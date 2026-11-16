//! nRF54L profile-scoped DPI tables. Register reads do not replace battery telemetry.
use super::{channel, feature::FeatureChannel, rapoo, trace, BackendId, Dpi, Target};
use crate::{device::{DpiStage, StageContext}, hid::HidInterface, win32::HidApi};
use std::time::{Duration, Instant};

const LENGTH: usize = 33;
const PROFILE: u32 = 0x010c;
const X: u32 = 648;
const Y: u32 = 712;
const ACTIVE: u32 = 664;
const BASE: u32 = 1536;
const STRIDE: u32 = 1024;
const SLOTS: usize = 7;

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
struct Config { profile:u8, x:Vec<u8>, y:Vec<u8> }
impl Config {
    fn decode(profile:u8, x:Vec<u8>, y:Vec<u8>) -> Result<Self,String> {
        if profile>7 || x.len()!=17 || y.len()!=14 || x[14]>=SLOTS as u8 || x[16]>x[14] {
            return Err("雷柏 DPI 配置无效".into());
        }
        for index in 0..=usize::from(x[14]) {
            if !(50..u16::MAX).contains(&Self::axis(&x,index))
                || !(50..u16::MAX).contains(&Self::axis(&y,index)) { return Err("雷柏 DPI 配置无效".into()); }
        }
        Ok(Self {profile,x,y})
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
    fn dpi(&self,status:&Dpi)->Dpi {
        let mut dpi=Dpi {current:status.current,current_y:status.current_y,profile:Some(u16::from(self.profile)),
            active_stage:self.matches_status(status).then_some(self.active()),configuration_token:Some(self.token()),
            stage_writable:self.matches_status(status),..Default::default()};
        for i in 0..=self.x[14] {
            dpi.stages.push(DpiStage {index:i,value:Self::axis(&self.x,usize::from(i))});
            dpi.stage_y.insert(i,Self::axis(&self.y,usize::from(i)));
        }
        dpi
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
            if started>=self.deadline {return Err("雷柏配置查询超时".into());}
            let p=packet(self.mode,command,address,count,data).ok_or("雷柏 DPI 配置无效")?;
            self.channel.send(&p)?;
            // Never resend a write when its acknowledgement is lost.
            for _ in 0..5 {
                if Instant::now()>=self.deadline {break;}
                self.channel.pause();
                let Ok(frame)=self.channel.receive() else {continue;};
                match reply(&frame,count) {
                    Some(Reply::Data(data))=>return Ok(data.to_vec()),
                    Some(Reply::Rejected)=>return Err("雷柏拒绝配置请求".into()),
                    _=>{}
                }
            }
            Err("雷柏配置应答不可用".into())
        })();
        trace::record(&self.interface,BackendId::Rapoo,if command==0xa4 {"read-register"} else {"set-stage"},
            address as u16,command,LENGTH,if result.is_ok(){LENGTH}else{0},started,
            if result.is_ok(){trace::Outcome::Ok}else{trace::Outcome::NoReply});
        result
    }
    fn read(&mut self,address:u32,count:usize)->Result<Vec<u8>,String> {self.query(0xa4,address,count,&[])}
    fn profile(&mut self)->Result<u8,String> {
        let p=self.read(PROFILE,1)?[0];
        if p>7 {Err("雷柏 DPI 配置无效".into())} else {Ok(p)}
    }
    fn config(&mut self)->Result<Config,String> {
        let profile=self.profile()?;
        let x=self.read(address(profile,X),17)?;
        let y=self.read(address(profile,Y),14)?;
        if self.profile()?!=profile {return Err("设备配置已变化，请刷新后重试".into());}
        // The documented ACK has no confirmed transaction/address echo. Repeat
        // independent tables in reverse order and cross-check live telemetry.
        if self.read(address(profile,Y),14)?!=y || self.read(address(profile,X),17)?!=x
            || self.profile()?!=profile {return Err("设备配置已变化，请刷新后重试".into());}
        Config::decode(profile,x,y)
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
        if after.active()!=stage.index || !before.same_tables(&after) {
            return Err("无法确认 DPI 档位切换".into());
        }
        Ok(after)
    }
}

fn open(target:&Target,interfaces:&[HidInterface])->Result<Session<Transport>,String> {
    let (output,feature)=endpoints(&target.interface,interfaces).ok_or("雷柏配置接口不可用")?;
    Ok(Session::new(Transport {output:FeatureChannel::open_output(output)?,feature:FeatureChannel::open(feature)?},&target.interface))
}

pub(super) fn read(api:&HidApi,target:&Target,status:&Dpi)->Result<Dpi,String> {
    let interfaces=super::enumeration::scan(api)?.interfaces;
    let key=channel::collection_group(&target.interface);let lock=channel::lock(&key);
    let _guard=lock.lock().unwrap_or_else(|p|p.into_inner());
    let config=open(target,&interfaces)?.config()?;
    if !config.matches_status(status) {return Err("设备档位或配置已变化，请刷新后重试".into());}
    Ok(config.dpi(status))
}

pub(super) fn switch(api:&HidApi,target:&Target,stage:DpiStage,context:StageContext)->Result<(),String> {
    let StageContext::Configuration(token)=context else {return Err("设备配置已变化，请刷新后重试".into());};
    let interfaces=super::enumeration::scan(api)?.interfaces;
    let key=channel::collection_group(&target.interface);let lock=channel::lock(&key);
    let _guard=lock.lock().unwrap_or_else(|p|p.into_inner());
    let (_,_,status)=rapoo::latest_status(target,None,Duration::from_millis(100))?;
    let after=open(target,&interfaces)?.switch(stage,token,&status)?;
    let (_,_,confirmed)=rapoo::latest_status(target,Some(Instant::now()),Duration::from_secs(4))?;
    if !after.matches_status(&confirmed) {return Err("无法确认 DPI 档位切换".into());}
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
                assert_eq!(p[2],0xa5);assert_eq!(addr,address(2,ACTIVE));assert_eq!(count,1);
                let data=p[8..8+count].to_vec();self.writes.push((addr,data.clone()));
                if self.accept {self.memory.insert(addr,data[0]);}
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
        let mut s=session();let c=s.config().unwrap();let d=c.dpi(&status());
        assert_eq!(d.stages.len(),7);assert_eq!(d.stages[6].value,25600);
        assert_eq!(d.stage_y[&6],30000);assert_eq!(d.profile,Some(2));assert!(d.stage_writable);
        assert!(!d.writable && !d.stage_editable);assert_eq!(c.x[15],0x5a);assert!(s.channel.writes.is_empty());
        s.channel.prefixed=false;
        s.channel.memory.insert(address(2,X),0x20);s.channel.memory.insert(address(2,X)+1,3);
        let d=s.config().unwrap().dpi(&status());assert_eq!(d.stages[0].value,800);assert_eq!(d.stages[1].value,800);
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
        assert!(!s.config().unwrap().dpi(&status()).stage_writable);assert!(s.channel.writes.is_empty());
    }
}
