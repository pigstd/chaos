pub trait BlockDevice: Send + Sync + std::any::Any {
    fn block_size(&self) -> usize;
    fn block_count(&self) -> u64;
    fn read_block(&self, block_id: u64, out: &mut [u8]) -> Result<(), &'static str>;
    fn write_block(&self, block_id: u64, data: &[u8]) -> Result<(), &'static str>;
    fn flush(&self) -> Result<(), &'static str>;
}
