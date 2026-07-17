use std::fmt::Debug;

pub(crate) trait TestResultExt<T> {
    fn test(self) -> anyhow::Result<T>;
    fn test_context(self, context: &str) -> anyhow::Result<T>;
}

impl<T, E: Debug> TestResultExt<T> for Result<T, E> {
    fn test(self) -> anyhow::Result<T> {
        self.map_err(|error| anyhow::anyhow!("test operation failed: {error:?}"))
    }

    fn test_context(self, context: &str) -> anyhow::Result<T> {
        self.map_err(|error| anyhow::anyhow!("{context}: {error:?}"))
    }
}

impl<T> TestResultExt<T> for Option<T> {
    fn test(self) -> anyhow::Result<T> {
        self.ok_or_else(|| anyhow::anyhow!("test expected a value"))
    }

    fn test_context(self, context: &str) -> anyhow::Result<T> {
        self.ok_or_else(|| anyhow::anyhow!("{context}"))
    }
}

pub(crate) trait TestResultErrorExt<E> {
    fn test_err(self) -> anyhow::Result<E>;
}

impl<T: Debug, E> TestResultErrorExt<E> for Result<T, E> {
    fn test_err(self) -> anyhow::Result<E> {
        match self {
            Ok(value) => anyhow::bail!("test expected an error, got {value:?}"),
            Err(error) => Ok(error),
        }
    }
}
