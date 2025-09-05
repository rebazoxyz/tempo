//! Hooks to format eyre reports with their source chain attached.

use std::fmt::Write as _;

/// Installs the hook as the global error report hook.
///
/// **NOTE**: It must be called before any `eyre::Report`s are constructed
/// to prevent the default handler from being installed.
///
/// # Errors
///
/// Calling this function after another handler has been installed will cause
/// an error.
pub fn install() -> eyre::Result<()> {
    eyre::set_hook(Box::new(|_| Box::new(ErrorHandler)))?;
    Ok(())
}

struct ErrorHandler;

impl eyre::EyreHandler for ErrorHandler {
    /// Copied directly from [`eyre::DefaultHandler`] because we can't construct
    /// and hence delegate to it.
    fn debug(
        &self,
        error: &(dyn std::error::Error + 'static),
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        use core::fmt::Write as _;

        if f.alternate() {
            return core::fmt::Debug::fmt(error, f);
        }

        write!(f, "{error}")?;

        if let Some(cause) = error.source() {
            write!(f, "\n\nCaused by:")?;
            let multiple = cause.source().is_some();
            for (n, error) in eyre::Chain::new(cause).enumerate() {
                writeln!(f)?;
                if multiple {
                    write!(indenter::indented(f).ind(n), "{error}")?;
                } else {
                    write!(indenter::indented(f), "{error}")?;
                }
            }
        }
        Result::Ok(())
    }

    fn display(
        &self,
        error: &(dyn std::error::Error + 'static),
        f: &mut core::fmt::Formatter<'_>,
    ) -> core::fmt::Result {
        let mut list = f.debug_list();
        let mut curr = Some(error);
        while let Some(curr_err) = curr {
            list.entry(&format_args!("{curr_err}"));
            curr = curr_err.source();
        }
        list.finish()?;
        Ok(())
    }
}
