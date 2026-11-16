//! Connection-specific protocol facts, kept separate from the shared Razer codec.
//! See PROTOCOLS.md for source revisions and evidence; none are locally hardware-verified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateEncoding {
    Legacy,
    Extended { commit: bool },
}

#[derive(Clone, Copy)]
pub struct Capabilities {
    pub max_dpi: u16,
    pub dpi_storage: u8,
    pub stage_storage: u8,
    pub rate: RateEncoding,
    pub rates: &'static [u16],
}
pub fn for_product(pid: u16) -> Option<Capabilities> {
    use RateEncoding::*;
    // These are sensor/transport limits, not proof that a command succeeded.
    // Live identity, device mode, complete reads and readback are required separately.
    let (max_dpi, dpi_storage, stage_storage, rate, rates): (_, _, _, _, &[u16]) = match pid {
        0x0078 => (16000, 0, 1, Legacy, &[125, 500, 1000]), // Viper wired
        0x0084 => (20000, 0, 1, Legacy, &[125, 500, 1000]), // DeathAdder V2
        0x00a5 => (30000, 1, 1, Legacy, &[125, 500, 1000]), // Viper V2 Pro wired
        0x00a6 => (30000, 1, 1, Extended { commit: false }, &[125, 500, 1000]),
        0x00b6 | 0x00b7 | 0x00b8 => (30000, 1, 1, Legacy, &[125, 500, 1000]),
        0x00c0 => (35000, 1, 1, Legacy, &[125, 500, 1000]), // Viper V3 Pro wired
        0x00c1 => (
            35000,
            1,
            1,
            Extended { commit: true },
            &[125, 500, 1000, 2000, 4000, 8000],
        ),
        _ => return None,
    };
    Some(Capabilities {
        max_dpi,
        dpi_storage,
        stage_storage,
        rate,
        rates,
    })
}
