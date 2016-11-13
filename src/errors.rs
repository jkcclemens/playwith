#![allow(redundant_closure)]

extern crate rusty_alfred;

error_chain! {
  links {
    rusty_alfred::errors::Error, rusty_alfred::errors::ErrorKind, RustyAlfredError;
  }
}
