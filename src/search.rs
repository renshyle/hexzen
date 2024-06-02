use std::cmp;

pub struct SearchResults {
    results: Vec<usize>,
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

        SearchResults::new(search_results)
    } else {
        None
    }
}

impl SearchResults {
    fn new(results: Vec<usize>) -> Option<SearchResults> {
        if results.is_empty() {
            return None;
        }

        Some(SearchResults { results, i: 0 })
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
