pub use fakemacros::command;
pub struct AppHandle;
pub struct State<'a, T>(pub &'a T);
impl<'a, T> std::ops::Deref for State<'a, T> { type Target = T; fn deref(&self) -> &T { self.0 } }
