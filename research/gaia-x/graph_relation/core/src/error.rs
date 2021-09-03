//
//! Copyright 2020 Alibaba Group Holding Limited.
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! http://www.apache.org/licenses/LICENSE-2.0
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.

pub type ParsePbResult<T> = Result<T, ParsePbError>;

#[derive(Debug, PartialEq)]
pub enum ParsePbError {
    InvalidPb(String),
}

impl std::fmt::Display for ParsePbError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ParsePbError::InvalidPb(s) => write!(f, "invalid protobuf: {}", s),
        }
    }
}

impl std::error::Error for ParsePbError {}

impl From<&str> for ParsePbError {
    fn from(e: &str) -> Self {
        ParsePbError::InvalidPb(e.into())
    }
}
