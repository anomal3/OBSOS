//! OBSOS на столе FossAi (`/opt/obsos/bin/obsos`).

use std::rc::Rc;

use obsos_fossai::window::{FossAi, Window};

fn main() {
    obsos_core::window::install(Box::new(FossAi));
    obsos_core::run(|title, width, height| Window::open(title, width, height).map(|w| Rc::new(w) as Rc<obsos_core::window::Window>))
}
