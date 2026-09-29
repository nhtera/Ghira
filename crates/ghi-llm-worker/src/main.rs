// SPDX-License-Identifier: Apache-2.0
//! Entry point of the LLM worker. Spawned by `ghi-llm` with `std::process`;
//! the stdio protocol and llama.cpp binding arrive in phase 6.

fn main() {
    println!("ghi-llm-worker {}", ghi_llm_worker::version());
}
