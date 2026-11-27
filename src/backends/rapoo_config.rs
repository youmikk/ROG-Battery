//! nRF54L profile-scoped DPI tables and polling rate. Register reads do not replace battery telemetry.
use super::{channel, feature::FeatureChannel, rapoo, trace, BackendId, Dpi, Target};
use crate::{device::{DpiStage, PollingRate, StageContext}, hid::HidInterface, win32::HidApi};
use std::time::{Duration, Instant};

const LENGTH: usize = 33;
const PROFILE: u32 = 0x010c;
const RATE: u32 = 640;
const X: u32 = 648;
const ENABLE_GEAR: u32 = 662;
const Y: u32 = 712;
const ACTIVE: u32 = 664;
const BASE: u32 = 1536;
const STRIDE: u32 = 1024;
const SLOTS: usize = 7;
const RETAIL_RATES: [(u8, u16); 7] = [
    (8, 125), (4, 250), (2, 500), (1, 1000), (132, 2000), (130, 4000), (129, 8000),
];
const CYBERCAFE_RATES: [(u8, u16); 4] = [(1, 1000), (132, 2000), (130, 4000), (129, 8000)];

fn rate_table(product_id: u16) -> &'static [(u8, u16)] {
    if rapoo::is_cybercafe(product_id) { &CYBERCAFE_RATES } else { &RETAIL_RATES }
}

fn max_dpi(product_id: u16) -> u16 {
    let Some(name) = rapoo::model(product_id) else { return 26_000; };
    if !name.contains("ESM") && (name.contains("MAX") || name.contains("V2") || name.contains("MASTER")) {
        45_000
    } else {
        26_000
    }
}

fn dpi_valid(product_id: u16, value: u16) -> bool {
    (50..=max_dpi(product_id)).contains(&value) && value % 50 == 0
}

fn dpi_options(product_id: u16) -> Vec<u16> {
    (50..=max_dpi(product_id)).step_by(50).collect()
}

fn address(profile: u8, offset: u32) -> u32 { BASE + u32::from(profile) * STRIDE + offset }

fn endpoints<'a>(status: &HidInterface, interfaces: &'a [HidInterface]) -> Option<(&'a HidInterface, &'a HidInterface)> {
    let group = channel::collection_group(status);
    // Unstructured paths cannot establish that separate collections belong to one receiver.
    if !group.contains('#') { return None; }
    let siblings: Vec<_> = interfaces.iter().filter(|i| i.vendor_id == status.vendor_id
        && i.product_id == status.product_id && i.usage_page == 0xff00
        && channel::collection_group(i) == group).collect();
    let outputs: Vec<_> = siblings.iter().copied().filter(|i| i.usage == 0x0e && i.output_len == LENGTH as u16).collect();
    let features: Vec<_> = siblings.iter().copied().filter(|i| i.usage == 0x0f && i.feature_len == LENGTH as u16).collect();
    (outputs.len() == 1 && features.len() == 1).then(|| (outputs[0],features[0]))
}

fn packet(mode: u8, command: u8, address: u32, count: usize, data: &[u8]) -> Option<[u8; LENGTH]> {
    if !matches!(mode,0xa5|0xff) || !(1..=25).contains(&count)
        || !matches!(command,0xa4|0xa5) || (command == 0xa5 && data.len() != count) { return None; }
    let mut p=[0;LENGTH];p[0]=6;p[1]=mode;p[2]=command;p[3]=count as u8;
    p[4..8].copy_from_slice(&address.to_le_bytes());
    if command == 0xa5 { p[8..8+count].copy_from_slice(data); }
    Some(p)
}

#[derive(Debug, PartialEq, Eq)]
enum Reply<'a> { Busy, Data(&'a [u8]), Rejected }
fn reply(frame: &[u8], count: usize) -> Option<Reply<'_>> {
    if frame.len() > LENGTH || !(1..=25).contains(&count) { return None; }
    let data=if frame.first()==Some(&8) {frame.get(1..)?} else {frame};
    match data.first()? {
        2 => Some(Reply::Busy),
        1 => Some(Reply::Data(data.get(4..4+count)?)),
        0 => Some(Reply::Rejected),
        _ => None,
    }
}

trait Channel {
    fn send(&mut self, packet: &[u8]) -> Result<(),String>;
    fn receive(&mut self) -> Result<Vec<u8>,String>;
    fn pause(&self) {}
}
struct Transport { output: FeatureChannel, feature: FeatureChannel }
impl Channel for Transport {
    fn send(&mut self, p: &[u8]) -> Result<(),String> { self.output.send_output(p,120) }
    fn receive(&mut self) -> Result<Vec<u8>,String> { self.feature.receive_raw(8,120) }
    fn pause(&self) { std::thread::sleep(Duration::from_millis(10)); }
}

#[derive(Clone,Debug,PartialEq,Eq)]
struct Config { profile:u8, x:Vec<u8>, y:Vec<u8>, rate:u8 }
impl Config {
    fn decode(profile:u8, x:Vec<u8>, y:Vec<u8>, rate:u8) -> Result<Self,String> {
        if profile>7 || x.len()!=17 || y.len()!=14 || x[14]>=SLOTS as u8 || x[16]>x[14] {
            return Err("读取到的设备 DPI 配置异常，请刷新重试".into());
        }
        for index in 0..=usize::from(x[14]) {
            if !(50..u16::MAX).contains(&Self::axis(&x,index))
                || !(50..u16::MAX).contains(&Self::axis(&y,index)) { return Err("读取到的设备 DPI 配置异常，请刷新重试".into()); }
        }
        Ok(Self {profile,x,y,rate})
    }
    fn axis(bytes:&[u8],index:usize)->u16 {u16::from_le_bytes([bytes[index*2],bytes[index*2+1]])}
    fn active(&self)->u8 {self.x[16]}
    fn token(&self)->u64 {
        let mut h=0xcbf29ce484222325u64;
        for v in std::iter::once(&self.profile).chain(&self.x).chain(&self.y) {
            h=(h ^ u64::from(*v)).wrapping_mul(0x100000001b3);
        }
        h
    }
    fn matches_status(&self, status:&Dpi)->bool {
        let i=usize::from(self.active());
        status.profile==Some(u16::from(self.profile)) && status.active_stage==Some(self.active())
            && status.current==Some(Self::axis(&self.x,i)) && status.current_y==Some(Self::axis(&self.y,i))
    }
    fn dpi(&self,status:&Dpi,product_id:u16)->Dpi {
        let matched=self.matches_status(status);
        let mut dpi=Dpi {current:status.current,current_y:status.current_y,values:dpi_options(product_id),
            profile:Some(u16::from(self.profile)),active_stage:matched.then_some(self.active()),
            configuration_token:Some(self.token()),stage_writable:matched,stage_editable:matched,
            max_stages:if matched { SLOTS as u8 } else { 0 },..Default::default()};
        for i in 0..=self.x[14] {
            dpi.stages.push(DpiStage {index:i,value:Self::axis(&self.x,usize::from(i))});
            dpi.stage_y.insert(i,Self::axis(&self.y,usize::from(i)));
        }
        dpi
    }
    fn polling_rate(&self,status:&Dpi,product_id:u16)->PollingRate {
        let table=rate_table(product_id);
        let current=table.iter().find(|&&(raw,_)|raw==self.rate).map(|&(_,hz)|hz);
        let values=if current.is_some() {table.iter().map(|&(_,hz)|hz).collect()} else {Vec::new()};
        PollingRate {current,values,writable:current.is_some() && self.matches_status(status),..Default::default()}
    }
    fn same_tables(&self,other:&Self)->bool {
        self.profile==other.profile && self.x[..16]==other.x[..16] && self.y==other.y
    }
}

struct Session<C> { channel:C, interface:HidInterface, mode:u8, deadline:Instant }
impl<C:Channel> Session<C> {
    fn new(channel:C, interface:&HidInterface)->Self {
        // The official paired PID catalog distinguishes wired 4xxx from receiver 1xxx.
        Self {channel,interface:interface.clone(),mode:if interface.product_id>=0x4000 {0xff} else {0xa5},
            deadline:Instant::now()+Duration::from_secs(2)}
    }
    fn query(&mut self,command:u8,address:u32,count:usize,data:&[u8])->Result<Vec<u8>,String> {
        let started=Instant::now();
        let result=(|| {
            if started>=self.deadline {return Err("等待设备配置响应超时，请唤醒后重试".into());}
            let p=packet(self.mode,command,address,count,data).ok_or("读取到的设备 DPI 配置异常，请刷新重试")?;
            self.channel.send(&p)?;
            // Never resend a write when its acknowledgement is lost.
            for _ in 0..5 {
                if Instant::now()>=self.deadline {break;}
                self.channel.pause();
                let Ok(frame)=self.channel.receive() else {continue;};
                match reply(&frame,count) {
                    Some(Reply::Data(data))=>return Ok(data.to_vec()),
                    Some(Reply::Rejected)=>return Err("设备暂未接受此次配置请求".into()),
                    _=>{}
                }
            }
            Err("暂未获取到设备配置响应，请重试".into())
        })();
        trace::record(&self.interface,BackendId::Rapoo,if command==0xa4 {"read-register"} else {"set-stage"},
            address as u16,command,LENGTH,if result.is_ok(){LENGTH}else{0},started,
            if result.is_ok(){trace::Outcome::Ok}else{trace::Outcome::NoReply});
        result
    }
    fn read(&mut self,address:u32,count:usize)->Result<Vec<u8>,String> {self.query(0xa4,address,count,&[])}
    fn profile(&mut self)->Result<u8,String> {
        let p=self.read(PROFILE,1)?[0];
        if p>7 {Err("读取到的设备 DPI 配置异常，请刷新重试".into())} else {Ok(p)}
    }
    fn config(&mut self)->Result<Config,String> {
        let profile=self.profile()?;
        let rate=self.read(address(profile,RATE),1)?[0];
        let x=self.read(address(profile,X),17)?;
        let y=self.read(address(profile,Y),14)?;
        if self.profile()?!=profile {return Err("设备配置已变化，请刷新后重试".into());}
        // The documented ACK has no confirmed transaction/address echo. Repeat
        // independent tables in reverse order and cross-check live telemetry.
        if self.read(address(profile,Y),14)?!=y || self.read(address(profile,X),17)?!=x
            || self.read(address(profile,RATE),1)?[0]!=rate || self.profile()?!=profile {
            return Err("设备配置已变化，请刷新后重试".into());
        }
        Config::decode(profile,x,y,rate)
    }
    fn switch(&mut self,stage:DpiStage,token:u64,status:&Dpi)->Result<Config,String> {
        let before=self.config()?;
        if before.token()!=token || !before.matches_status(status)
            || stage.index>before.x[14] || Config::axis(&before.x,usize::from(stage.index))!=stage.value {
            return Err("设备档位或配置已变化，请刷新后重试".into());
        }
        if before.active()==stage.index {return Ok(before);}
        if self.profile()?!=before.profile {return Err("设备配置已变化，请刷新后重试".into());}
        let _=self.query(0xa5,address(before.profile,ACTIVE),1,&[stage.index]);
        self.deadline=Instant::now()+Duration::from_secs(2);
        let after=self.config()?;
        if after.active()!=stage.index || !before.same_tables(&after) || after.rate!=before.rate {
            return Err("无法确认 DPI 档位切换".into());
        }
        Ok(after)
    }
    fn write_sorted_stages(&mut self,before:&Config,mut next:Vec<(u16,u16,bool)>)->Result<Config,String> {
        next.sort_by_key(|&(x,y,_)|(x,y));
        let new_count=next.len();
        if !(1..=SLOTS).contains(&new_count) {return Err("设备不支持新增 DPI 档位".into());}
        let target_max=(new_count-1) as u8;
        let expected_active=next.iter().position(|&(_,_,active)|active)
            .unwrap_or_else(||usize::from(before.active()).min(new_count-1)) as u8;
        if self.profile()?!=before.profile {return Err("设备配置已变化，请刷新后重试".into());}
        let mut expected_x=before.x.clone();
        let mut expected_y=before.y.clone();
        expected_x[14]=target_max;
        expected_x[16]=expected_active;
        for (slot,&(x,y,_)) in next.iter().enumerate() {
            let off=slot*2;
            expected_x[off..off+2].copy_from_slice(&x.to_le_bytes());
            expected_y[off..off+2].copy_from_slice(&y.to_le_bytes());
            if expected_x[off..off+2]!=before.x[off..off+2] {
                let _=self.query(0xa5,address(before.profile,X)+off as u32,2,&expected_x[off..off+2]);
            }
            if expected_y[off..off+2]!=before.y[off..off+2] {
                let _=self.query(0xa5,address(before.profile,Y)+off as u32,2,&expected_y[off..off+2]);
            }
        }
        if expected_active!=before.active() {
            let _=self.query(0xa5,address(before.profile,ACTIVE),1,&[expected_active]);
        }
        if target_max!=before.x[14] {
            let _=self.query(0xa5,address(before.profile,ENABLE_GEAR),1,&[target_max]);
        }
        self.deadline=Instant::now()+Duration::from_secs(2);
        let after=self.config()?;
        if after.profile!=before.profile || after.x!=expected_x || after.y!=expected_y || after.rate!=before.rate {
            return Err("无法确认设备已保存设置，请刷新后检查".into());
        }
        Ok(after)
    }
    fn edit(&mut self,stage:DpiStage,token:u64,status:&Dpi,expected_y:u16,x:u16,y:u16)->Result<Config,String> {
        let pid=self.interface.product_id;
        let before=self.config()?;
        if before.token()!=token || !before.matches_status(status)
            || stage.index>before.x[14]
            || Config::axis(&before.x,usize::from(stage.index))!=stage.value
            || Config::axis(&before.y,usize::from(stage.index))!=expected_y {
            return Err("设备档位或配置已变化，请刷新后重试".into());
        }
        if !dpi_valid(pid,x) || !dpi_valid(pid,y) {
            return Err("DPI 不在设备报告的可用范围内".into());
        }
        if stage.value==x && expected_y==y {return Ok(before);}
        let next:Vec<_>=(0..=usize::from(before.x[14])).map(|i|{
            if i==usize::from(stage.index) { (x,y,i==usize::from(before.active())) }
            else { (Config::axis(&before.x,i),Config::axis(&before.y,i),i==usize::from(before.active())) }
        }).collect();
        self.write_sorted_stages(&before,next)
    }
    fn add_stage(&mut self,token:u64,status:&Dpi,x:u16,y:u16)->Result<Config,String> {
        let pid=self.interface.product_id;
        let before=self.config()?;
        if before.token()!=token || !before.matches_status(status) {
            return Err("设备档位或配置已变化，请刷新后重试".into());
        }
        let count=usize::from(before.x[14])+1;
        if count>=SLOTS {return Err("DPI 档位数量已达上限".into());}
        if !dpi_valid(pid,x) || !dpi_valid(pid,y) {
            return Err("DPI 不在设备报告的可用范围内".into());
        }
        let mut next:Vec<_>=(0..count).map(|i|{
            (Config::axis(&before.x,i),Config::axis(&before.y,i),i==usize::from(before.active()))
        }).collect();
        next.push((x,y,false));
        self.write_sorted_stages(&before,next)
    }
    fn remove_stage(&mut self,stage:DpiStage,token:u64,status:&Dpi)->Result<Config,String> {
        let before=self.config()?;
        if before.token()!=token || !before.matches_status(status)
            || stage.index>before.x[14]
            || Config::axis(&before.x,usize::from(stage.index))!=stage.value {
            return Err("设备档位或配置已变化，请刷新后重试".into());
        }
        let count=usize::from(before.x[14])+1;
        if count<=1 {return Err("至少需保留 1 个 DPI 档位".into());}
        if stage.index==before.active() {return Err("当前使用档位无法删除".into());}
        let next:Vec<_>=(0..count).filter(|&i|i!=usize::from(stage.index)).map(|i|{
            (Config::axis(&before.x,i),Config::axis(&before.y,i),i==usize::from(before.active()))
        }).collect();
        self.write_sorted_stages(&before,next)
    }
    fn set_rate(&mut self,status:&Dpi,hz:u16)->Result<Config,String> {
        let pid=self.interface.product_id;
        let code=rate_table(pid).iter().find(|&&(_,rate)|rate==hz).map(|&(raw,_)|raw)
            .ok_or("回报率不在此连接的可用范围内")?;
        let before=self.config()?;
        if !before.matches_status(status) {return Err("设备档位或配置已变化，请刷新后重试".into());}
        if before.rate==code {return Ok(before);}
        if self.profile()?!=before.profile {return Err("设备配置已变化，请刷新后重试".into());}
        let _=self.query(0xa5,address(before.profile,RATE),1,&[code]);
        self.deadline=Instant::now()+Duration::from_secs(2);
        let after=self.config()?;
        if after.rate!=code || !before.same_tables(&after) || after.active()!=before.active() {
            return Err("无法确认设备已保存设置，请刷新后检查".into());
        }
        Ok(after)
    }
}

fn open(target:&Target,interfaces:&[HidInterface])->Result<Session<Transport>,String> {
    let (output,feature)=endpoints(target.hid()?,interfaces).ok_or("暂未找到设备配置通道，请检查连接后重试")?;
    Ok(Session::new(Transport {output:FeatureChannel::open_output(output)?,feature:FeatureChannel::open(feature)?},target.hid()?))
}

fn sync_after(target:&Target,after:&Config) {
    let active=usize::from(after.active());
    rapoo::sync_status_dpi(
        target,
        after.profile,
        after.active(),
        Config::axis(&after.x,active),
        Config::axis(&after.y,active),
    );
}

pub(super) fn read(api:&HidApi,target:&Target,status:&Dpi)->Result<(Dpi,PollingRate),String> {
    let interfaces=super::enumeration::scan(api)?.interfaces;
    let pid=target.hid()?.product_id;
    let key=channel::collection_group(target.hid()?);let lock=channel::lock(&key);
    let _guard=lock.lock().unwrap_or_else(|p|p.into_inner());
    let config=open(target,&interfaces)?.config()?;
    // 遥测状态未与 EEPROM 对齐时仍返回只读档位与回报率，避免整页退化为空白状态页。
    Ok((config.dpi(status,pid),config.polling_rate(status,pid)))
}

pub(super) fn switch(api:&HidApi,target:&Target,stage:DpiStage,context:StageContext)->Result<(),String> {
    let StageContext::Configuration(token)=context else {return Err("设备配置已变化，请刷新后重试".into());};
    let interfaces=super::enumeration::scan(api)?.interfaces;
    let key=channel::collection_group(target.hid()?);let lock=channel::lock(&key);
    let _guard=lock.lock().unwrap_or_else(|p|p.into_inner());
    let (_,_,status)=rapoo::latest_status(target,None,Duration::from_millis(100))?;
    let after=open(target,&interfaces)?.switch(stage,token,&status)?;
    let (_,_,confirmed)=rapoo::latest_status(target,Some(Instant::now()),Duration::from_secs(4))?;
    if !after.matches_status(&confirmed) {return Err("无法确认 DPI 档位切换".into());}
    Ok(())
}

pub(super) fn edit(api:&HidApi,target:&Target,stage:DpiStage,context:StageContext,expected_y:u16,x:u16,y:u16)->Result<(),String> {
    let StageContext::Configuration(token)=context else {return Err("设备配置已变化，请刷新后重试".into());};
    let interfaces=super::enumeration::scan(api)?.interfaces;
    let key=channel::collection_group(target.hid()?);let lock=channel::lock(&key);
    let _guard=lock.lock().unwrap_or_else(|p|p.into_inner());
    let (_,_,status)=rapoo::latest_status(target,None,Duration::from_millis(100))?;
    let after=open(target,&interfaces)?.edit(stage,token,&status,expected_y,x,y)?;
    sync_after(target,&after);
    Ok(())
}

pub(super) fn add_stage(api:&HidApi,target:&Target,context:StageContext,x:u16,y:u16)->Result<(),String> {
    let StageContext::Configuration(token)=context else {return Err("设备配置已变化，请刷新后重试".into());};
    let interfaces=super::enumeration::scan(api)?.interfaces;
    let key=channel::collection_group(target.hid()?);let lock=channel::lock(&key);
    let _guard=lock.lock().unwrap_or_else(|p|p.into_inner());
    let (_,_,status)=rapoo::latest_status(target,None,Duration::from_millis(100))?;
    let after=open(target,&interfaces)?.add_stage(token,&status,x,y)?;
    sync_after(target,&after);
    Ok(())
}

pub(super) fn remove_stage(api:&HidApi,target:&Target,stage:DpiStage,context:StageContext)->Result<(),String> {
    let StageContext::Configuration(token)=context else {return Err("设备配置已变化，请刷新后重试".into());};
    let interfaces=super::enumeration::scan(api)?.interfaces;
    let key=channel::collection_group(target.hid()?);let lock=channel::lock(&key);
    let _guard=lock.lock().unwrap_or_else(|p|p.into_inner());
    let (_,_,status)=rapoo::latest_status(target,None,Duration::from_millis(100))?;
    let after=open(target,&interfaces)?.remove_stage(stage,token,&status)?;
    sync_after(target,&after);
    Ok(())
}

pub(super) fn set_rate(api:&HidApi,target:&Target,hz:u16)->Result<(),String> {
    let interfaces=super::enumeration::scan(api)?.interfaces;
    let key=channel::collection_group(target.hid()?);let lock=channel::lock(&key);
    let _guard=lock.lock().unwrap_or_else(|p|p.into_inner());
    let (_,_,status)=rapoo::latest_status(target,None,Duration::from_millis(100))?;
    let _after=open(target,&interfaces)?.set_rate(&status,hz)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap,VecDeque};
    fn interface(instance:&str,col:u8,usage:u16)->HidInterface {
        HidInterface {path:format!(r"\\?\hid#vid_24ae&pid_1415&mi_00&col{col:02x}#{instance}&00{col:02x}#{{hid}}"),
            vendor_id:0x24ae,product_id:0x1415,usage_page:0xff00,usage,
            input_len:if usage==2 {19}else{0},output_len:if usage==14 {33}else{0},
            feature_len:if usage==15 {33}else{0},product:String::new(),serial:"same-serial".into()}
    }
    struct Reports {
        memory:BTreeMap<u32,u8>,pending:VecDeque<Vec<u8>>,writes:Vec<(u32,Vec<u8>)>,
        accept:bool,lose_ack:bool,corrupt_neighbor:bool,change_profile:bool,change_during_read:bool,
        prefixed:bool,reads:usize,all_reads_same:bool,
    }
    impl Reports {
        fn new()->Self {
            let mut memory=BTreeMap::new();memory.insert(PROFILE,2);
            for (offset,values) in [(X,[400u16,800,1600,3200,6400,12800,25600]),
                (Y,[400u16,800,1600,3200,6400,12800,30000])] {
                for (i,v) in values.iter().enumerate() {
                    for (b,value) in v.to_le_bytes().into_iter().enumerate() {
                        memory.insert(address(2,offset)+(i*2+b) as u32,value);
                    }
                }
            }
            memory.insert(address(2,X)+14,6);memory.insert(address(2,X)+15,0x5a);
            memory.insert(address(2,ACTIVE),1);
            memory.insert(address(2,RATE),8);
            Self {memory,pending:VecDeque::new(),writes:Vec::new(),accept:true,lose_ack:false,
                corrupt_neighbor:false,change_profile:false,change_during_read:false,prefixed:true,reads:0,all_reads_same:false}
        }
        fn bytes(&self,address:u32,count:usize)->Vec<u8> {
            (0..count).map(|i|*self.memory.get(&(address+i as u32)).unwrap_or(&0)).collect()
        }
        fn frame(&self,data:&[u8])->Vec<u8> {
            let mut r=vec![0;LENGTH];let offset=usize::from(self.prefixed);
            if self.prefixed {r[0]=8;}r[offset]=1;
            r[offset+4..offset+4+data.len()].copy_from_slice(data);r
        }
    }
    impl Channel for Reports {
        fn send(&mut self,p:&[u8])->Result<(),String> {
            assert_eq!(p.len(),LENGTH);assert_eq!(p[0],6);assert!(matches!(p[1],0xa5|0xff));
            let addr=u32::from_le_bytes(p[4..8].try_into().unwrap());let count=usize::from(p[3]);
            self.pending.clear();self.pending.push_back(p.to_vec());
            self.pending.push_back(vec![8,2,0,0,0]);
            let data=if p[2]==0xa4 {
                self.reads+=1;
                let data=if self.all_reads_same {vec![2;count]}else{self.bytes(addr,count)};
                if self.change_during_read && self.reads==2 {self.memory.insert(PROFILE,3);}
                data
            } else {
                assert_eq!(p[2],0xa5);
                let data=p[8..8+count].to_vec();self.writes.push((addr,data.clone()));
                if self.accept {
                    for (offset, byte) in data.iter().enumerate() {
                        self.memory.insert(addr + offset as u32, *byte);
                    }
                }
                if self.change_profile {self.memory.insert(PROFILE,3);}
                if self.corrupt_neighbor {self.memory.insert(address(2,Y)+12,1);}
                if self.lose_ack {return Err("lost ACK".into());}
                data
            };
            self.pending.push_back(self.frame(&data));Ok(())
        }
        fn receive(&mut self)->Result<Vec<u8>,String> {self.pending.pop_front().ok_or("silent".into())}
    }
    fn session()->Session<Reports> {Session::new(Reports::new(),&interface("receiver-a",2,2))}
    fn status()->Dpi {Dpi {current:Some(800),current_y:Some(800),active_stage:Some(1),profile:Some(2),..Default::default()}}

    #[test]
    fn sibling_endpoints_preserve_receiver_instance_and_usb_interface() {
        let status=interface("a",2,2);
        let siblings=vec![interface("a",5,14),interface("a",6,15),interface("b",5,14),interface("b",6,15)];
        let (out,feat)=endpoints(&status,&siblings).unwrap();
        assert_eq!(out.path,siblings[0].path);assert_eq!(feat.path,siblings[1].path);
        assert!(endpoints(&status,&siblings[1..]).is_none());
        let mut wrong=siblings.clone();wrong[1].path=wrong[1].path.replace("mi_00","mi_01");
        assert!(endpoints(&status,&wrong).is_none());
        let mut wrong=siblings.clone();wrong[0].output_len=65;assert!(endpoints(&status,&wrong).is_none());
        let mut duplicate=siblings.clone();duplicate.push(siblings[0].clone());assert!(endpoints(&status,&duplicate).is_none());
        assert_ne!(Target::new(BackendId::Rapoo,&status,0).key,
            Target::new(BackendId::Rapoo,&interface("b",2,2),0).key);
    }
    #[test]
    fn framing_separates_wired_wireless_and_profile_addresses() {
        let p=packet(0xa5,0xa4,address(0,X),17,&[]).unwrap();
        assert_eq!(&p[..8],&[6,0xa5,0xa4,17,0x88,8,0,0]);
        let p=packet(0xff,0xa5,address(2,ACTIVE),1,&[6]).unwrap();
        assert_eq!(&p[..9],&[6,0xff,0xa5,1,0x98,0x10,0,0,6]);
        assert!(packet(0xba,0xa5,0,1,&[1]).is_none());assert!(packet(0xa5,0xa4,0,26,&[]).is_none());
        assert!(packet(0xa5,0xa5,0,2,&[1]).is_none());
        let mut wired=interface("a",2,2);wired.product_id=0x4615;
        assert_eq!(Session::new(Reports::new(),&wired).mode,0xff);
    }
    #[test]
    fn reply_parser_accepts_both_windows_layouts_and_rejects_echo_busy_or_truncated_data() {
        assert_eq!(reply(&[1,0,0,0,7],1),Some(Reply::Data(&[7])));
        assert_eq!(reply(&[8,1,0,0,0,7],1),Some(Reply::Data(&[7])));
        assert_eq!(reply(&[8,2],1),Some(Reply::Busy));assert_eq!(reply(&[0],1),Some(Reply::Rejected));
        assert_eq!(reply(&[8,1,0,0,0],1),None);
        assert_eq!(reply(&packet(0xa5,0xa4,PROFILE,1,&[]).unwrap(),1),None);
        assert_eq!(reply(&[1;34],1),None);
    }
    #[test]
    fn config_reads_all_seven_xy_slots_and_keeps_reserved_bytes_and_duplicates() {
        let mut s=session();let c=s.config().unwrap();let d=c.dpi(&status(),0x1415);
        assert_eq!(d.stages.len(),7);assert_eq!(d.stages[6].value,25600);
        assert_eq!(d.stage_y[&6],30000);assert_eq!(d.profile,Some(2));assert!(d.stage_writable);
        assert!(!d.writable && d.stage_editable);assert!(d.values.contains(&850) && !d.values.contains(&26050));
        assert_eq!(c.x[15],0x5a);assert!(s.channel.writes.is_empty());
        let rate=c.polling_rate(&status(),0x1415);
        assert_eq!(rate.current,Some(125));assert!(rate.writable);
        assert_eq!(rate.values,vec![125,250,500,1000,2000,4000,8000]);
        s.channel.prefixed=false;
        s.channel.memory.insert(address(2,X),0x20);s.channel.memory.insert(address(2,X)+1,3);
        let d=s.config().unwrap().dpi(&status(),0x1415);assert_eq!(d.stages[0].value,800);assert_eq!(d.stages[1].value,800);
    }
    #[test]
    fn switching_sends_only_active_byte_then_verifies_full_tables_even_without_ack() {
        let mut s=session();let before=s.config().unwrap();s.channel.lose_ack=true;
        let after=s.switch(DpiStage {index:6,value:25600},before.token(),&status()).unwrap();
        assert_eq!(s.channel.writes,vec![(address(2,ACTIVE),vec![6])]);
        assert!(before.same_tables(&after));assert_eq!(after.active(),6);
        let changed=Dpi {current:Some(25600),current_y:Some(30000),active_stage:Some(6),profile:Some(2),..Default::default()};
        assert!(after.matches_status(&changed));assert!(!after.matches_status(&status()));
    }
    #[test]
    fn editing_stage_dpi_updates_xy_slots_and_preserves_other_settings() {
        let mut s=session();let before=s.config().unwrap();
        let stage=DpiStage {index:1,value:800};
        let after=s.edit(stage,before.token(),&status(),800,1200,1400).unwrap();
        assert_eq!(s.channel.writes,vec![
            (address(2,X)+2,1200u16.to_le_bytes().to_vec()),
            (address(2,Y)+2,1400u16.to_le_bytes().to_vec()),
        ]);
        assert_eq!(Config::axis(&after.x,1),1200);
        assert_eq!(Config::axis(&after.y,1),1400);
        assert_eq!(after.rate,before.rate);
        assert!(s.edit(stage,before.token(),&status(),800,825,825).is_err());
        assert!(s.edit(stage,before.token(),&status(),800,26050,26050).is_err());
    }
    #[test]
    fn setting_polling_rate_writes_rate_register_and_respects_cybercafe_limits() {
        let mut s=session();
        let after=s.set_rate(&status(),4000).unwrap();
        assert_eq!(s.channel.writes,vec![(address(2,RATE),vec![130])]);
        assert_eq!(after.rate,130);
        let st=Dpi {current:Some(800),current_y:Some(800),active_stage:Some(1),profile:Some(2),..Default::default()};
        assert_eq!(after.polling_rate(&st,0x1415).current,Some(4000));
        assert!(s.set_rate(&status(),1234).is_err());
        let mut cyber=Session::new(Reports::new(),&interface("c",2,2));
        cyber.interface.product_id=0x4633;
        assert!(cyber.set_rate(&status(),125).is_err());
        cyber.channel.memory.insert(address(2,RATE),1);
        assert_eq!(cyber.config().unwrap().polling_rate(&status(),0x4633).values,vec![1000,2000,4000,8000]);
    }
    #[test]
    fn adding_and_editing_stages_sorts_low_to_high_and_tracks_active_stage() {
        let mut s=session();
        s.channel.memory.insert(address(2,X)+14,0);
        s.channel.memory.insert(address(2,ACTIVE),0);
        s.channel.memory.insert(address(2,X),0x20);s.channel.memory.insert(address(2,X)+1,3);
        s.channel.memory.insert(address(2,Y),0x20);s.channel.memory.insert(address(2,Y)+1,3);
        let st800=Dpi {current:Some(800),current_y:Some(800),active_stage:Some(0),profile:Some(2),..Default::default()};
        let before=s.config().unwrap();
        let after_add=s.add_stage(before.token(),&st800,400,400).unwrap();
        assert_eq!(after_add.x[14],1);
        assert_eq!(Config::axis(&after_add.x,0),400);
        assert_eq!(Config::axis(&after_add.x,1),800);
        assert_eq!(after_add.active(),1);

        let st800_at1=Dpi {current:Some(800),current_y:Some(800),active_stage:Some(1),profile:Some(2),..Default::default()};
        let after_edit=s.edit(DpiStage {index:0,value:400},after_add.token(),&st800_at1,400,1600,1600).unwrap();
        assert_eq!(Config::axis(&after_edit.x,0),800);
        assert_eq!(Config::axis(&after_edit.x,1),1600);
        assert_eq!(after_edit.active(),0);

        assert!(s.remove_stage(DpiStage {index:0,value:800},after_edit.token(),&st800).is_err());
        let after_rm=s.remove_stage(DpiStage {index:1,value:1600},after_edit.token(),&st800).unwrap();
        assert_eq!(after_rm.x[14],0);
        assert_eq!(Config::axis(&after_rm.x,0),800);
        assert_eq!(after_rm.active(),0);
        assert!(s.remove_stage(DpiStage {index:0,value:800},after_rm.token(),&st800).is_err());
    }
    #[test]
    fn stale_token_profile_axes_or_active_stage_cannot_authorize_a_write() {
        let mut s=session();let c=s.config().unwrap();let stage=DpiStage {index:6,value:25600};
        assert!(s.switch(stage,c.token()^1,&status()).is_err());
        let mut wrong=status();wrong.profile=Some(0);assert!(s.switch(stage,c.token(),&wrong).is_err());
        wrong=status();wrong.current_y=Some(1600);assert!(s.switch(stage,c.token(),&wrong).is_err());
        wrong=status();wrong.active_stage=Some(0);assert!(s.switch(stage,c.token(),&wrong).is_err());
        assert!(s.switch(DpiStage {index:6,value:800},c.token(),&status()).is_err());
        assert!(s.channel.writes.is_empty());
    }
    #[test]
    fn ignored_write_changed_profile_and_neighbor_corruption_are_errors_without_rollback() {
        for mode in 0..3 {
            let mut s=session();let c=s.config().unwrap();
            match mode {0=>s.channel.accept=false,1=>s.channel.change_profile=true,_=>s.channel.corrupt_neighbor=true}
            assert!(s.switch(DpiStage {index:6,value:25600},c.token(),&status()).is_err());
            assert_eq!(s.channel.writes.len(),1);
        }
    }
    #[test]
    fn changing_profile_invalid_counts_and_replayed_constant_response_do_not_produce_settings() {
        let mut s=session();s.channel.change_during_read=true;assert!(s.config().is_err());
        let mut s=session();s.channel.memory.insert(address(2,X)+14,7);assert!(s.config().is_err());
        let mut s=session();s.channel.memory.insert(address(2,ACTIVE),7);assert!(s.config().is_err());
        let mut s=session();s.channel.memory.insert(address(2,Y),255);s.channel.memory.insert(address(2,Y)+1,255);
        assert!(s.config().is_err());
        let mut s=session();s.channel.all_reads_same=true;
        assert!(!s.config().unwrap().dpi(&status(),0x1415).stage_writable);assert!(s.channel.writes.is_empty());
    }
    #[test]
    fn mismatched_status_keeps_stages_and_polling_rate_read_only_instead_of_dropping_them() {
        let mut s=session();
        let config=s.config().unwrap();
        let mut stale=status();
        stale.active_stage=Some(0);
        let dpi=config.dpi(&stale,0x1415);
        let rate=config.polling_rate(&stale,0x1415);
        assert_eq!(dpi.stages.len(),7);
        assert!(!dpi.stage_writable);
        assert!(!dpi.stage_editable);
        assert_eq!(rate.current,Some(125));
        assert!(!rate.writable);
    }
}
