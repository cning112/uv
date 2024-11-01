use std::error::Error;
use std::fmt::{Display, Formatter};
use std::ops::Bound;

use version_ranges::Ranges;

use crate::{Operator, Prerelease, Version, VersionSpecifier, VersionSpecifiers};

/// The conversion between PEP 440 [`VersionSpecifier`] and version-ranges
/// [`VersionRangesSpecifier`] failed.
#[derive(Debug)]
pub enum VersionRangesSpecifierError {
    /// The `~=` operator requires at least two release segments
    InvalidTildeEquals(VersionSpecifier),
}

impl Error for VersionRangesSpecifierError {}

impl Display for VersionRangesSpecifierError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTildeEquals(specifier) => {
                write!(
                    f,
                    "The `~=` operator requires at least two release segments: `{specifier}`"
                )
            }
        }
    }
}

/// A range of versions that can be used to satisfy a requirement.
#[derive(Debug)]
pub struct VersionRangesSpecifier(Ranges<Version>);

impl VersionRangesSpecifier {
    /// Returns an iterator over the bounds of the [`VersionRangesSpecifier`].
    pub fn iter(&self) -> impl Iterator<Item = (&Bound<Version>, &Bound<Version>)> {
        self.0.iter()
    }

    /// Return the bounding [`Ranges`] of the [`VersionRangesSpecifier`].
    pub fn bounding_range(&self) -> Option<(Bound<&Version>, Bound<&Version>)> {
        self.0.bounding_range()
    }
}

impl From<Ranges<Version>> for VersionRangesSpecifier {
    fn from(range: Ranges<Version>) -> Self {
        VersionRangesSpecifier(range)
    }
}

impl From<VersionRangesSpecifier> for Ranges<Version> {
    /// Convert a PubGrub specifier to a range of versions.
    fn from(specifier: VersionRangesSpecifier) -> Self {
        specifier.0
    }
}

impl VersionRangesSpecifier {
    /// Convert [`VersionSpecifiers`] to a PubGrub-compatible version range, using PEP 440
    /// semantics.
    pub fn from_pep440_specifiers(
        specifiers: &VersionSpecifiers,
    ) -> Result<Self, VersionRangesSpecifierError> {
        let mut range = Ranges::full();
        for specifier in specifiers.iter() {
            range = range.intersection(&Self::from_pep440_specifier(specifier)?.into());
        }
        Ok(Self(range))
    }

    /// Convert the [`VersionSpecifier`] to a PubGrub-compatible version range, using PEP 440
    /// semantics.
    pub fn from_pep440_specifier(
        specifier: &VersionSpecifier,
    ) -> Result<Self, VersionRangesSpecifierError> {
        let ranges = match specifier.operator() {
            Operator::Equal => {
                let version = specifier.version().clone();
                Ranges::singleton(version)
            }
            Operator::ExactEqual => {
                let version = specifier.version().clone();
                Ranges::singleton(version)
            }
            Operator::NotEqual => {
                let version = specifier.version().clone();
                Ranges::singleton(version).complement()
            }
            Operator::TildeEqual => {
                let [rest @ .., last, _] = specifier.version().release() else {
                    return Err(VersionRangesSpecifierError::InvalidTildeEquals(
                        specifier.clone(),
                    ));
                };
                let upper = Version::new(rest.iter().chain([&(last + 1)]))
                    .with_epoch(specifier.version().epoch())
                    .with_dev(Some(0));
                let version = specifier.version().clone();
                Ranges::from_range_bounds(version..upper)
            }
            Operator::LessThan => {
                let version = specifier.version().clone();
                if version.any_prerelease() {
                    Ranges::strictly_lower_than(version)
                } else {
                    // Per PEP 440: "The exclusive ordered comparison <V MUST NOT allow a
                    // pre-release of the specified version unless the specified version is itself a
                    // pre-release."
                    Ranges::strictly_lower_than(version.with_min(Some(0)))
                }
            }
            Operator::LessThanEqual => {
                let version = specifier.version().clone();
                Ranges::lower_than(version)
            }
            Operator::GreaterThan => {
                // Per PEP 440: "The exclusive ordered comparison >V MUST NOT allow a post-release of
                // the given version unless V itself is a post release."
                let version = specifier.version().clone();
                if let Some(dev) = version.dev() {
                    Ranges::higher_than(version.with_dev(Some(dev + 1)))
                } else if let Some(post) = version.post() {
                    Ranges::higher_than(version.with_post(Some(post + 1)))
                } else {
                    Ranges::strictly_higher_than(version.with_max(Some(0)))
                }
            }
            Operator::GreaterThanEqual => {
                let version = specifier.version().clone();
                Ranges::higher_than(version)
            }
            Operator::EqualStar => {
                let low = specifier.version().clone().with_dev(Some(0));
                let mut high = low.clone();
                if let Some(post) = high.post() {
                    high = high.with_post(Some(post + 1));
                } else if let Some(pre) = high.pre() {
                    high = high.with_pre(Some(Prerelease {
                        kind: pre.kind,
                        number: pre.number + 1,
                    }));
                } else {
                    let mut release = high.release().to_vec();
                    *release.last_mut().unwrap() += 1;
                    high = high.with_release(release);
                }
                Ranges::from_range_bounds(low..high)
            }
            Operator::NotEqualStar => {
                let low = specifier.version().clone().with_dev(Some(0));
                let mut high = low.clone();
                if let Some(post) = high.post() {
                    high = high.with_post(Some(post + 1));
                } else if let Some(pre) = high.pre() {
                    high = high.with_pre(Some(Prerelease {
                        kind: pre.kind,
                        number: pre.number + 1,
                    }));
                } else {
                    let mut release = high.release().to_vec();
                    *release.last_mut().unwrap() += 1;
                    high = high.with_release(release);
                }
                Ranges::from_range_bounds(low..high).complement()
            }
        };

        Ok(Self(ranges))
    }

    /// Convert the [`VersionSpecifiers`] to a PubGrub-compatible version range, using release-only
    /// semantics.
    ///
    /// Assumes that the range will only be tested against versions that consist solely of release
    /// segments (e.g., `3.12.0`, but not `3.12.0b1`).
    ///
    /// These semantics are used for testing Python compatibility (e.g., `requires-python` against
    /// the user's installed Python version). In that context, it's more intuitive that `3.13.0b0`
    /// is allowed for projects that declare `requires-python = ">3.13"`.
    ///
    /// See: <https://github.com/pypa/pip/blob/a432c7f4170b9ef798a15f035f5dfdb4cc939f35/src/pip/_internal/resolution/resolvelib/candidates.py#L540>
    pub fn from_release_specifiers(
        specifiers: &VersionSpecifiers,
    ) -> Result<Self, VersionRangesSpecifierError> {
        let mut range = Ranges::full();
        for specifier in specifiers.iter() {
            range = range.intersection(&Self::from_release_specifier(specifier)?.into());
        }
        Ok(Self(range))
    }

    /// Convert the [`VersionSpecifier`] to a PubGrub-compatible version range, using release-only
    /// semantics.
    ///
    /// Assumes that the range will only be tested against versions that consist solely of release
    /// segments (e.g., `3.12.0`, but not `3.12.0b1`).
    ///
    /// These semantics are used for testing Python compatibility (e.g., `requires-python` against
    /// the user's installed Python version). In that context, it's more intuitive that `3.13.0b0`
    /// is allowed for projects that declare `requires-python = ">3.13"`.
    ///
    /// See: <https://github.com/pypa/pip/blob/a432c7f4170b9ef798a15f035f5dfdb4cc939f35/src/pip/_internal/resolution/resolvelib/candidates.py#L540>
    pub fn from_release_specifier(
        specifier: &VersionSpecifier,
    ) -> Result<Self, VersionRangesSpecifierError> {
        let ranges = match specifier.operator() {
            Operator::Equal => {
                let version = specifier.version().only_release();
                Ranges::singleton(version)
            }
            Operator::ExactEqual => {
                let version = specifier.version().only_release();
                Ranges::singleton(version)
            }
            Operator::NotEqual => {
                let version = specifier.version().only_release();
                Ranges::singleton(version).complement()
            }
            Operator::TildeEqual => {
                let [rest @ .., last, _] = specifier.version().release() else {
                    return Err(VersionRangesSpecifierError::InvalidTildeEquals(
                        specifier.clone(),
                    ));
                };
                let upper = Version::new(rest.iter().chain([&(last + 1)]));
                let version = specifier.version().only_release();
                Ranges::from_range_bounds(version..upper)
            }
            Operator::LessThan => {
                let version = specifier.version().only_release();
                Ranges::strictly_lower_than(version)
            }
            Operator::LessThanEqual => {
                let version = specifier.version().only_release();
                Ranges::lower_than(version)
            }
            Operator::GreaterThan => {
                let version = specifier.version().only_release();
                Ranges::strictly_higher_than(version)
            }
            Operator::GreaterThanEqual => {
                let version = specifier.version().only_release();
                Ranges::higher_than(version)
            }
            Operator::EqualStar => {
                let low = specifier.version().only_release();
                let high = {
                    let mut high = low.clone();
                    let mut release = high.release().to_vec();
                    *release.last_mut().unwrap() += 1;
                    high = high.with_release(release);
                    high
                };
                Ranges::from_range_bounds(low..high)
            }
            Operator::NotEqualStar => {
                let low = specifier.version().only_release();
                let high = {
                    let mut high = low.clone();
                    let mut release = high.release().to_vec();
                    *release.last_mut().unwrap() += 1;
                    high = high.with_release(release);
                    high
                };
                Ranges::from_range_bounds(low..high).complement()
            }
        };
        Ok(Self(ranges))
    }
}
