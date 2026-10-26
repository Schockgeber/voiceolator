//! Real-time thread priority.
//!
//! On Windows, audio threads register with MMCSS ("Pro Audio" task) so that the
//! scheduler keeps servicing them on time while a game saturates the CPU. This raises
//! priority only for our three small audio threads; it does not affect the game.

#[cfg(windows)]
pub fn promote_current_thread() -> bool {
    use windows::core::w;
    use windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW;
    let mut idx = 0u32;
    unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut idx).is_ok() }
}

#[cfg(not(windows))]
pub fn promote_current_thread() -> bool {
    false
}
