use std::cmp::{self, Ordering};

pub struct SearchResults {
    results: Vec<usize>,
    match_size: usize,
    i: usize,
}

pub fn search(buffer: &[u8], input: &str) -> Option<SearchResults> {
    let bytes = if input.starts_with('/') {
        Ok(input.strip_prefix('/').unwrap().as_bytes().to_vec())
    } else {
        hex::decode(input)
    };

    if let Ok(bytes) = bytes {
        let search_results = memchr::memmem::find_iter(buffer, &bytes).collect::<Vec<usize>>();

        SearchResults::new(search_results, bytes.len())
    } else {
        None
    }
}

impl SearchResults {
    fn new(results: Vec<usize>, match_size: usize) -> Option<SearchResults> {
        if results.is_empty() {
            return None;
        }

        Some(SearchResults {
            results,
            match_size,
            i: 0,
        })
    }

    /// Given an `offset` into the file, returns `None` if the search did not match the byte at `offset`. If a match was
    /// found, returns the number of bytes starting at `offset` that matched, at least 1.
    pub fn match_len(&self, offset: usize) -> Option<usize> {
        self.results
            .binary_search_by(|&x| {
                if offset >= x && offset < x + self.match_size {
                    Ordering::Equal
                } else if offset < x {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            })
            .map(|x| self.match_size + self.results[x] - offset)
            .ok()
    }

    pub fn len(&self) -> usize {
        self.results.len()
    }

    pub fn next(&mut self) -> usize {
        self.i = cmp::min(self.len() - 1, self.i + 1);
        self.results[self.i]
    }

    pub fn prev(&mut self) -> usize {
        self.i = self.i.saturating_sub(1);
        self.results[self.i]
    }

    pub fn result(&self) -> usize {
        self.results[self.i]
    }

    pub fn idx(&self) -> usize {
        self.i
    }
}
