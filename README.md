# Calleen

[![Crates.io](https://img.shields.io/crates/v/calleen.svg)](https://crates.io/crates/calleen)
[![Documentation](https://docs.rs/calleen/badge.svg)](https://docs.rs/calleen)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

I've been writing production Rust applications for quite a few years now, and in every new project I find myself replicating
certain patterns. This library provides what I would consider "best practices" when sending an HTTP request, and parsing its response.

To avoid [the XY problem](https://xyproblem.info/), let me first describe the problems I wanted to solve:
1. `serde`/`serde_json` don't retain the raw data when they fail to deserialize. This means that you'll get error logs that say "failed to deserialize" but have no insight into what the bad input was. As an individual, it is easy to work around this. But, as a team, it slips through pretty frequently, especially with engineers new to Rust and on-call log debugging.
2. Retry logic built in to the call layer that is HTTP-response-code aware -- I've been in many projects where we have ad-hoc retry logic at the callsite. And sometimes it knows not to retry e.g. 4xx errors, and only to retry 5xx errors. Sometimes it doesn't know. 
3. Critical failures and non-actionable were not disambiguated, meaning you could get paged when on-call for a third party 5xx response. Something you as an engineer can do nothing about!

This library addresses these three concerns primarily.
1. `calleen` retains the raw response, so if deserialization fails, the error log contains the raw input. This does have some memory overhead, but _it is worth it_. As somebody who has been paged at 1am for a serde deserialization failure many times in his life, I will always spend these bytes.
2. Centralized retry strategy definitions which are status-code aware and reasonably customizable.
3. Disambiguation among various failure modes -- `tracing::warn!()` on typically non-actionable responses like 5xx, `tracing::error!()` on `4xx` or failure to deserialize response types, which are typically actionable and urgent. For companies I've worked in, we typically page on `error!()` logs, so this triggers our PagerDuty.
