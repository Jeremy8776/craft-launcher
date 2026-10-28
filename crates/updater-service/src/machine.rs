//! What is installed for all users on this PC. The service only updates what is already there.

use artcraft_launcher_core::CraftApp;

/// The version of `app` installed per-machine, if any.
pub trait Machine {
    fn installed(&self, app: &CraftApp) -> Option<semver::Version>;
}

/// Reads Windows Installer's own record of who a product is installed for, not the Add/Remove
/// Programs entry. That entry can sit in HKLM for a copy that is really installed for one user
/// (every app on the development PC is like that), and updating such a copy as if it were
/// per-machine is exactly what the launcher's `INSTALLATION.md` warns against.
///
/// A product is per-machine if, and only if, Windows Installer keeps its record under
/// `Installer\UserData\S-1-5-18` (the machine's own slot). That key is writable only by SYSTEM
/// and Administrators, so a normal user can't make a product look per-machine. The uninstall
/// entries are used only to find which product codes belong to the app. The version comes from the
/// machine record itself.
#[cfg(windows)]
pub struct Registry;

/// Windows Installer's packed form of a product code, as used in its registry keys: the first three
/// groups reversed, then every remaining pair of digits swapped. `None` if `code` is not a GUID.
pub fn packed_product_code(code: &str) -> Option<String> {
    let digits: Vec<char> = code.strip_prefix('{')?.strip_suffix('}')?.chars().collect();
    let groups = [(0, 8), (9, 13), (14, 18), (19, 23), (24, 36)];
    if digits.len() != 36
        || [8, 13, 18, 23].iter().any(|&i| digits.get(i) != Some(&'-'))
        || !digits.iter().all(|c| *c == '-' || c.is_ascii_hexdigit())
    {
        return None;
    }
    let mut out = String::with_capacity(32);
    for (n, (from, to)) in groups.into_iter().enumerate() {
        let part: Vec<char> = digits.get(from..to)?.to_vec();
        if n < 3 {
            out.extend(part.iter().rev().map(char::to_ascii_uppercase));
        } else {
            for pair in part.chunks(2) {
                out.extend(pair.iter().rev().map(char::to_ascii_uppercase));
            }
        }
    }
    Some(out)
}

#[cfg(windows)]
impl Registry {
    /// The version Windows Installer has recorded for `product_code` in the machine's own slot.
    fn machine_version(product_code: &str) -> Option<semver::Version> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

        let path = format!(
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData\S-1-5-18\Products\{}\InstallProperties",
            packed_product_code(product_code)?
        );
        let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY).ok()?;
        let text: String = key.get_value("DisplayVersion").ok()?;
        artcraft_launcher_core::version::parse(&text)
    }
}

#[cfg(windows)]
impl Machine for Registry {
    fn installed(&self, app: &CraftApp) -> Option<semver::Version> {
        use artcraft_launcher_core::version::compare;
        use artcraft_launcher_native::detect::windows::msi_installs;

        msi_installs(app).iter().filter_map(|m| Self::machine_version(&m.product_code)).max_by(compare)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_a_product_code_the_way_windows_installer_does() {
        // CADCraft's code and the key Windows Installer really uses for it (seen in the registry).
        assert_eq!(packed_product_code("{C54DC3ED-D352-46E1-A885-8DD61648CB65}").as_deref(), Some("DE3CD45C253D1E648A58D86D6184BC56"));
        assert_eq!(packed_product_code("{c54dc3ed-d352-46e1-a885-8dd61648cb65}").as_deref(), Some("DE3CD45C253D1E648A58D86D6184BC56"));
    }

    #[test]
    fn refuses_anything_that_is_not_a_product_code() {
        for bad in [
            "",
            "{}",
            "C54DC3ED-D352-46E1-A885-8DD61648CB65",
            "{C54DC3ED-D352-46E1-A885-8DD61648CB6}",
            "{C54DC3ED-D352-46E1-A885-8DD61648CB6G}",
            "{C54DC3ED_D352_46E1_A885_8DD61648CB65}",
            "..\\..\\x",
        ] {
            assert_eq!(packed_product_code(bad), None, "{bad:?}");
        }
    }

    /// Real registry, this PC. The apps here are registered in HKLM's Add/Remove Programs but
    /// Windows Installer records them for one user, so the service must not see them as per-machine.
    #[cfg(windows)]
    #[test]
    fn an_app_installed_for_one_user_is_not_per_machine_even_with_a_machine_uninstall_entry() {
        use artcraft_launcher_native::detect::windows::msi_installs;
        let mut checked = 0;
        for app in artcraft_launcher_core::SUITE {
            for m in msi_installs(&app) {
                let machine_record = Registry::machine_version(&m.product_code);
                let user_record = user_slot_has(&m.product_code);
                if user_record {
                    assert_eq!(machine_record, None, "{} is recorded for a user and must not count as per-machine", app.id);
                    assert_eq!(Registry.installed(&app), None, "{}", app.id);
                    checked += 1;
                }
            }
        }
        eprintln!("checked {checked} apps recorded for a single user");
    }

    /// Whether any account-specific slot (not the machine's `S-1-5-18`) holds the product.
    #[cfg(windows)]
    fn user_slot_has(product_code: &str) -> bool {
        use winreg::RegKey;
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};
        let Some(packed) = packed_product_code(product_code) else { return false };
        let base = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData";
        let Ok(root) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(base, KEY_READ | KEY_WOW64_64KEY) else { return false };
        root.enum_keys()
            .flatten()
            .filter(|sid| sid.starts_with("S-1-5-21-"))
            .any(|sid| root.open_subkey_with_flags(format!(r"{sid}\Products\{packed}\InstallProperties"), KEY_READ | KEY_WOW64_64KEY).is_ok())
    }
}
