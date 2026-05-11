use toradb_core::SegmentId;

#[derive(Debug)]
pub struct Segment {
    pub id: SegmentId,
}

#[derive(Debug, Default)]
pub struct SegmentManager {
    segments: Vec<Segment>,
    next_id: SegmentId,
}

impl SegmentManager {
    pub fn new() -> Self {
        let mut mgr = Self::default();
        for _ in 0..4 {
            mgr.create_segment();
        }
        mgr
    }

    pub fn create_segment(&mut self) -> SegmentId {
        let id = self.next_id;
        self.next_id += 1;
        self.segments.push(Segment { id });
        id
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }
}
