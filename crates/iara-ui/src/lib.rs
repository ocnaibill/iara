//! Interface do Iara. O modelo (`model`) é puro e testável sem GTK; os widgets ficam atrás da feature `gtk-ui`.
pub mod meters;
pub mod model;

#[cfg(feature = "gtk-ui")]
pub mod app;
#[cfg(feature = "gtk-ui")]
pub mod backend;
