use super::*;

pub(crate) struct EditPermissions {
    private: Allocation,
    explicit: Vec<u32>,
}

impl EditPermissions {
    pub(crate) fn capture(file: &File) -> Result<Option<Self>, String> {
        let mut flags = 0;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetVolumeInformationByHandleW(
                file: Handle,
                volume: *mut u16,
                volume_len: u32,
                serial: *mut u32,
                max_component: *mut u32,
                flags: *mut u32,
                filesystem: *mut u16,
                filesystem_len: u32,
            ) -> i32;
        }
        if unsafe {
            GetVolumeInformationByHandleW(
                file.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut flags,
                std::ptr::null_mut(),
                0,
            )
        } == 0
        {
            return Err(error("read filesystem permission capabilities"));
        }
        if flags & 8 == 0 {
            return Ok(None);
        }
        let saved = descriptor(file)?;
        if output::control(saved.0)? & 0x1000 != 0 {
            return Ok(None);
        }
        let (mut present, mut acl, mut defaulted) = (0, std::ptr::null_mut(), 0);
        if unsafe { GetSecurityDescriptorDacl(saved.0, &mut present, &mut acl, &mut defaulted) }
            == 0
        {
            return Err(error("read existing archive DACL"));
        }
        if present == 0 || acl.is_null() {
            return Self::from_recipe(Vec::new()).map(Some);
        }
        // Preserve every explicit ACE, including object-specific and deny ACEs.
        // Windows derives the inherited part from the actual live parent.
        let header = unsafe { std::slice::from_raw_parts(acl.cast::<u8>(), 8) };
        let size = u16::from_le_bytes([header[2], header[3]]) as usize;
        let count = u16::from_le_bytes([header[4], header[5]]);
        if size < 8 {
            return Err("Invalid archive DACL".into());
        }
        let mut explicit = vec![0u32; size.div_ceil(4)];
        let raw = explicit.as_mut_ptr().cast::<u8>();
        unsafe {
            std::ptr::copy_nonoverlapping(header.as_ptr(), raw, 8);
        }
        let (mut used, mut kept) = (8usize, 0u16);
        for i in 0..count {
            let mut ace = std::ptr::null_mut();
            if unsafe { GetAce(acl, i as u32, &mut ace) } == 0 {
                return Err(error("read archive ACE"));
            }
            let header = unsafe { std::slice::from_raw_parts(ace.cast::<u8>(), 4) };
            let len = u16::from_le_bytes([header[2], header[3]]) as usize;
            if len < 4 || used + len > size {
                return Err("Invalid archive ACE".into());
            }
            if header[1] & 0x10 == 0 {
                unsafe {
                    std::ptr::copy_nonoverlapping(ace.cast::<u8>(), raw.add(used), len);
                }
                used += len;
                kept += 1;
            }
        }
        unsafe {
            std::ptr::write_unaligned(raw.add(2).cast::<u16>(), used as u16);
            std::ptr::write_unaligned(raw.add(4).cast::<u16>(), kept);
        }
        Ok(Some(Self {
            private: protected_descriptor(&current_sid(&token()?)?)?,
            explicit,
        }))
    }

    pub(crate) fn recipe(&self) -> Vec<u32> {
        self.explicit.clone()
    }

    pub(crate) fn from_recipe(explicit: Vec<u32>) -> Result<Self, String> {
        #[link(name = "advapi32")]
        extern "system" {
            fn IsValidAcl(acl: Handle) -> i32;
        }
        if explicit.is_empty() {
            return Ok(Self {
                private: protected_descriptor(&current_sid(&token()?)?)?,
                explicit,
            });
        }
        if explicit.len() < 2 || explicit.len() > 16384 {
            return Err("Invalid saved archive ACL".into());
        }
        let p = explicit.as_ptr().cast::<u8>();
        let size = unsafe { std::ptr::read_unaligned(p.add(2).cast::<u16>()) as usize };
        if size < 8 || size > explicit.len() * 4 || unsafe { IsValidAcl(p as Handle) } == 0 {
            return Err("Invalid saved archive ACL".into());
        }
        Ok(Self {
            private: protected_descriptor(&current_sid(&token()?)?)?,
            explicit,
        })
    }

    pub(crate) fn protect(&self, file: &File) -> Result<(), String> {
        if unsafe { NtSetSecurityObject(file.as_raw_handle(), 4 | 0x80000000, self.private.0) } < 0
        {
            return Err("Cannot protect unpublished archive permissions".into());
        }
        Ok(())
    }

    pub(crate) fn inherit(&self, file: &File) -> Result<(), String> {
        let result = unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                1,
                4 | 0x20000000,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                if self.explicit.is_empty() {
                    std::ptr::null_mut()
                } else {
                    self.explicit.as_ptr() as Handle
                },
                std::ptr::null_mut(),
            )
        };
        if result != 0 {
            return Err(format!(
                "Apply current archive parent permissions: {}",
                io::Error::from_raw_os_error(result as i32)
            ));
        }
        Ok(())
    }
}
