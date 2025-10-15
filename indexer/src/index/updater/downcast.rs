use std::{any::Any, sync::Arc};
use crate::index::store::Store;

pub fn downcast_arc<T: Any + Send + Sync>(
    arc: Arc<dyn Store + Send + Sync>,
) -> Result<Arc<T>, Arc<dyn Store + Send + Sync>> {
    if arc.as_any().is::<T>() {
        unsafe {
            let ptr = Arc::into_raw(arc) as *const T;
            Ok(Arc::from_raw(ptr))
        }
    } else {
        Err(arc)
    }
}