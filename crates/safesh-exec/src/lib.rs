//! brush-core embedding: runs a script with policy gates on every exec and file open.

#[cfg(test)]
mod tests {
    #[test]
    fn patched_brush_core_has_check_open() {
        fn has<T: brush_core::extensions::ProcessLauncher>() {}
        has::<brush_core::extensions::DefaultProcessLauncher>();
        let _ = <brush_core::extensions::DefaultProcessLauncher as brush_core::extensions::ProcessLauncher>::check_open;
    }
}
