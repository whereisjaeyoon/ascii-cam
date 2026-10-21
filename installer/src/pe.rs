// Inspect exports without loading or executing a DLL in the installer process.
#![cfg_attr(not(windows), allow(dead_code))]

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

pub fn validate_image(data: &[u8], dll: bool) -> Result<(), &'static str> {
    let valid = (|| -> Option<()> {
        if data.get(..2)? != b"MZ" {
            return None;
        }
        let pe = u32_at(data, 0x3c)? as usize;
        let header = data.get(pe..pe.checked_add(24)?)?;
        let characteristics = u16_at(header, 22)?;
        if &header[..4] != b"PE\0\0"
            || u16_at(header, 4)? != 0x8664
            || (characteristics & 0x2000 != 0) != dll
        {
            return None;
        }
        if characteristics & 0x2 == 0 {
            return None;
        }
        let size = u16_at(header, 20)? as usize;
        if size < 112 || u16_at(header, 6)? == 0 {
            return None;
        }
        let start = pe.checked_add(24)?;
        let optional = data.get(start..start.checked_add(size)?)?;
        if u16_at(optional, 0)? != 0x20b {
            return None;
        }
        Some(())
    })();
    valid.ok_or("The package contains an invalid or non-x64 Windows executable/DLL.")
}

pub fn validate(data: &[u8]) -> Result<(), &'static str> {
    validate_image(data, true)?;
    validate_inner(data).ok_or("Expected a valid x64 COM DLL exporting DllGetClassObject, DllCanUnloadNow, DllRegisterServer and DllUnregisterServer. hello_append.dll is not a COM DLL.")
}

fn validate_inner(data: &[u8]) -> Option<()> {
    if data.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(data, 0x3c)? as usize;
    // Entire optional header is bounded before field offsets are added.
    let header = data.get(pe..pe.checked_add(24)?)?;
    if &header[..4] != b"PE\0\0"
        || u16_at(header, 4)? != 0x8664
        || u16_at(header, 22)? & 0x2000 == 0
    {
        return None;
    }
    let sections = u16_at(header, 6)? as usize;
    let optional_size = u16_at(header, 20)? as usize;
    let optional_start = pe.checked_add(24)?;
    let optional = data.get(optional_start..optional_start.checked_add(optional_size)?)?;
    if u16_at(optional, 0)? != 0x20b || u32_at(optional, 108)? == 0 {
        return None;
    }
    let export_rva = u32_at(optional, 112)?;
    if export_rva == 0 {
        return None;
    }
    let section_start = optional_start.checked_add(optional_size)?;
    let section_table =
        data.get(section_start..section_start.checked_add(sections.checked_mul(40)?)?)?;
    let offset = |rva: u32, length: usize| -> Option<usize> {
        let length = u32::try_from(length).ok()?;
        for section in section_table.chunks_exact(40) {
            let va = u32_at(section, 12)?;
            let raw_size = u32_at(section, 16)?;
            let raw = u32_at(section, 20)?;
            if rva >= va {
                let delta = rva.checked_sub(va)?;
                if delta.checked_add(length)? <= raw_size {
                    let at = raw.checked_add(delta)? as usize;
                    data.get(at..at.checked_add(length as usize)?)?;
                    return Some(at);
                }
            }
        }
        None
    };
    let exports = offset(export_rva, 40)?;
    let count = u32_at(data, exports + 24)? as usize;
    if count > 65536 {
        return None;
    }
    let names = offset(u32_at(data, exports + 32)?, count.checked_mul(4)?)?;
    let mut found = [false; 4];
    let required: [&[u8]; 4] = [
        b"DllGetClassObject",
        b"DllCanUnloadNow",
        b"DllRegisterServer",
        b"DllUnregisterServer",
    ];
    for index in 0..count {
        let name_rva = u32_at(data, names + index * 4)?;
        let start = offset(name_rva, 1)?;
        let rest = data
            .get(start..)?
            .get(..data.len().saturating_sub(start).min(256))?;
        let name = &rest[..rest.iter().position(|b| *b == 0)?];
        for (index, required) in required.iter().enumerate() {
            found[index] |= name == *required;
        }
    }
    found.iter().all(|value| *value).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn put16(data: &mut [u8], at: usize, value: u16) {
        data[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    fn put32(data: &mut [u8], at: usize, value: u32) {
        data[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut d = vec![0; 2048];
        d[..2].copy_from_slice(b"MZ");
        put32(&mut d, 0x3c, 128);
        d[128..132].copy_from_slice(b"PE\0\0");
        put16(&mut d, 132, 0x8664);
        put16(&mut d, 134, 1);
        put16(&mut d, 148, 240);
        put16(&mut d, 150, 0x2002);
        put16(&mut d, 152, 0x20b);
        put32(&mut d, 260, 16);
        put32(&mut d, 264, 0x1000);
        put32(&mut d, 404, 0x1000);
        put32(&mut d, 408, 1536);
        put32(&mut d, 412, 512);
        put32(&mut d, 536, 4);
        put32(&mut d, 544, 0x1040);
        let mut cursor = 640;
        for (i, name) in [
            "DllGetClassObject",
            "DllCanUnloadNow",
            "DllRegisterServer",
            "DllUnregisterServer",
        ]
        .iter()
        .enumerate()
        {
            put32(&mut d, 576 + i * 4, 0x1000 + (cursor - 512) as u32);
            d[cursor..cursor + name.len()].copy_from_slice(name.as_bytes());
            cursor += name.len() + 1;
        }
        d
    }
    #[test]
    fn rejects_wrong_binary_kind_for_clients_and_runtime_dlls() {
        let mut d = fixture();
        put16(&mut d, 150, 0x2002);
        assert!(validate_image(&d, true).is_ok());
        assert!(validate_image(&d, false).is_err());
        put16(&mut d, 150, 2);
        assert!(validate_image(&d, false).is_ok());
        assert!(validate_image(&d, true).is_err());
        put16(&mut d, 132, 0x14c);
        assert!(validate_image(&d, false).is_err());
    }
    #[test]
    fn accepts_expected_exports() {
        assert!(validate(&fixture()).is_ok());
    }
    #[test]
    fn rejects_missing_export_and_x86() {
        let mut d = fixture();
        d[640] = b'X';
        assert!(validate(&d).is_err());
        let mut d = fixture();
        put16(&mut d, 132, 0x14c);
        assert!(validate(&d).is_err());
    }
    #[test]
    fn truncated_or_invalid_input_does_not_panic() {
        let d = fixture();
        for end in 0..d.len() {
            let _ = validate(&d[..end]);
        }
        assert!(validate(b"not a DLL").is_err());
        let mut d = fixture();
        put32(&mut d, 0x3c, u32::MAX);
        assert!(validate(&d).is_err());
    }
}
