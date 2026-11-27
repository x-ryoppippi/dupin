//! How extensions name SF Symbols, which Dupin draws as icons: `sf:moon.fill` where an image
//! file's path could stand.

/// What sets a symbol's name apart from a path in a package.
pub const SYMBOL_PREFIX: &str = "sf:";
/// The longest SF Symbol name accepted; the system's are well under it.
pub const MAX_SYMBOL_NAME: usize = 64;

/// Whether `name` is shaped as an SF Symbol's name: 1 to [`MAX_SYMBOL_NAME`] ASCII letters,
/// digits and dots, starting and ending with a letter or digit, with no empty part. Whether
/// the system has a symbol of that name is known only when it is drawn.
pub fn is_symbol_name(name: &str) -> bool {
    (1..=MAX_SYMBOL_NAME).contains(&name.len())
        && name
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}
