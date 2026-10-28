//! Spacesheep for Pointman: Talk through the node. A page's Session tab on spacesheep.dev messages the agent
//! session that published it; the extension listens for each agent here whose session published a page, with
//! Spacesheep's own CLI (`spacesheep talk listen`), and posts each message in the agent's thread, which wakes it (or
//! wakes the agents the message starts by calling). main.rs runs it under the node; the parts are here so the tests
//! can drive them.

pub mod cli;
pub mod pages;
pub mod said;
pub mod threads;
pub mod words;
