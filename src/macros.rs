//! Crate-wide declarative macros.
//!
//! Kept in its own module so models.rs (and any future macro-using files)
//! don't carry the macro definition in their sloc count -- which matters
//! because the maintainability index penalises log(loc) heavily and we want
//! models.rs to be judged on its actual schema content, not boilerplate
//! generators.

/// Generate boilerplate impls for fieldless enums whose variants map 1:1 to
/// canonical user-facing strings. Two forms:
///
/// 1. minimal (only `as_str`):
///    enum_strings!(EdgeType, { DependsOn => "depends_on", ... });
///
/// 2. full (`as_str` + `all_names` + `parse`, with an `err:` format string
///    used by the parse error path; the format string must have two `{}`
///    placeholders for the bad value and the joined list of valid names):
///    enum_strings!(Estimator, err: "invalid estimator '{}' in artifact. valid: {}",
///        { Mean => "mean", ... });
///
/// the `parse` arm joins `all_names()` with " | " to render the valid-set
/// hint, so adding/removing a variant only requires editing the table.
#[macro_export]
macro_rules! enum_strings {
    (
        $Enum:ident, err: $err_fmt:literal,
        { $( $Variant:ident => $name:literal ),* $(,)? }
    ) => {
        impl $Enum {
            pub fn as_str(&self) -> &'static str {
                match self {
                    $( $Enum::$Variant => $name, )*
                }
            }
            pub fn all_names() -> &'static [&'static str] {
                &[$( $name, )*]
            }
            pub fn parse(s: &str) -> anyhow::Result<Self> {
                match s {
                    $( $name => Ok($Enum::$Variant), )*
                    other => Err(anyhow::anyhow!(
                        $err_fmt, other, Self::all_names().join(" | ")
                    )),
                }
            }
        }
    };
    (
        $Enum:ident,
        { $( $Variant:ident => $name:literal ),* $(,)? }
    ) => {
        impl $Enum {
            pub fn as_str(&self) -> &'static str {
                match self {
                    $( $Enum::$Variant => $name, )*
                }
            }
        }
    };
}
