use rand::Rng;
use rand::rngs::ThreadRng;
use std::num::ParseIntError;

pub const MAX_SEED: u32 = u32::MAX;

pub struct SeedIterator {
    seeds: Option<Vec<u32>>,
    rng: ThreadRng,
}

impl SeedIterator {
    pub fn new(seeds: Option<Vec<u32>>) -> Self {
        let rng = rand::rng();
        Self { seeds, rng }
    }
}

impl Iterator for SeedIterator {
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(ref mut seeds) = self.seeds {
            return seeds.pop();
        }

        Some(self.rng.random_range(0..MAX_SEED))
    }
}

/// Parse seeds from a file
/// Read line per line the provided file and extract seeds from it
pub fn parse_seeds_file(path: &str) -> Result<Option<Vec<u32>>, Box<dyn std::error::Error>> {
    let file = std::fs::read_to_string(path)?;
    let seeds: Vec<u32> = file
        .lines()
        .map(|line| line.parse())
        .collect::<Result<_, ParseIntError>>()?;

    for seed in &seeds {
        if seed > &MAX_SEED {
            return Err(format!("Seed {} is greater than {}", seed, MAX_SEED).into());
        }
    }

    Ok(Some(seeds))
}

/// Merges user-provided seeds with seeds loaded from a file if specified.
///
/// This function combines two sources of seeds:
/// 1. A `Vec<u32>` provided by the user (`seeds` argument).
/// 2. Optional seeds loaded from a file identified by the `file_seeds_path`.
///
/// If both sources are provided, the seeds are merged, with file-based seeds being appended to the
/// user-defined seeds. If only one source is available, it is returned as-is. If neither source is provided,
/// the result is `None`.
///
/// # Arguments
///
/// * `seeds` - An `Option<Vec<u32>>` containing user-defined seeds. Can be `None` if no seeds are provided by the user.
/// * `file_seeds_path` - A reference to an `Option<String>` specifying the path to a file containing seeds.
///   If `None`, no file-based seeds are loaded.
///
/// # Returns
///
/// Returns a `Result` containing:
/// * `Ok(Some(Vec<u32>))` - If any seeds (user-defined or file-based) are successfully merged.
/// * `Ok(None)` - If no seeds are provided by either source.
/// * `Err(Box<dyn std::error::Error>)` - If an error occurs while parsing the file specified by `file_seeds_path`.
///
/// # Errors
///
/// This function returns an error if:
/// * The `file_seeds_path` is `Some` and an error occurs while attempting to read or parse the seeds file.
///
/// # Dependencies
///
/// This function calls an auxiliary function `parse_seeds_file`, which is expected to handle the
/// logic for reading and parsing the file at the given path into an `Option<Vec<u32>>`. Ensure
/// that `parse_seeds_file` is implemented correctly and returns appropriate errors.
///
/// # Notes
///
/// The order of seeds in the resulting `Vec<u32>` will maintain the order of `seeds` first,
/// followed by the order of seeds from the file (if any).
pub fn merge_user_defined_seeds(
    seeds: Option<Vec<u32>>,
    file_seeds_path: &Option<String>,
) -> Result<Option<Vec<u32>>, Box<dyn std::error::Error>> {
    let file_seeds = match file_seeds_path {
        Some(path) => parse_seeds_file(path)?,
        None => None,
    };

    // merge seeds
    let seeds = match seeds {
        Some(mut seeds) => {
            if let Some(file_seeds) = file_seeds {
                seeds.extend(file_seeds);
            }
            Some(seeds)
        }
        None => file_seeds,
    };

    Ok(seeds)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_seed_iterator() {
        let seeds = vec![1, 2, 3];
        let mut iter = SeedIterator::new(Some(seeds));
        assert_eq!(iter.next(), Some(3));
        assert_eq!(iter.next(), Some(2));
        assert_eq!(iter.next(), Some(1));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_seed_iterator_empty() {
        let iter = SeedIterator::new(None);
        for i in iter.take(10) {
            println!("{}", i);
        }
    }
}
