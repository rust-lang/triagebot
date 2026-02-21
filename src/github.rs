use crate::errors::{AssignmentError, UserError};
use crate::team_data::TeamClient;
use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use chrono::{DateTime, FixedOffset, Utc};
use futures::{FutureExt, future::BoxFuture};
use itertools::Itertools;
use octocrab::models::{Author, AuthorAssociation};
use regex::Regex;
use reqwest::header::{AUTHORIZATION, USER_AGENT};
use reqwest::{Client, Request, RequestBuilder, Response, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::{
    fmt,
    time::{Duration, SystemTime},
};
use tracing as log;

pub(crate) mod client;
pub(crate) mod event;
pub(crate) mod issue;
pub(crate) mod issue_query;
pub(crate) mod issue_repository;
pub(crate) mod repository;
pub(crate) mod utils;
mod webhook;

pub use webhook::webhook;

pub type UserId = u64;
pub type PullRequestNumber = u64;
