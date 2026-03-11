library(magrittr)
library(tidyverse)
library(fs)
library(here)


# here::here("devcluser")
devcluster_benches <- 
  fs::dir_ls(type = "file", "benchmark_log/devcluster/", glob = "*.csv") %>%
  enframe() %>% 
  mutate(
    results = value %>% map(. %>% readr::read_csv(lazy = FALSE, num_threads = 1))
  )
devcluster_benches %>% 
  unnest(results) %>% 
  View()

# # BUG in readr!
# readr::read_csv(
#   num_threads = 0,
#   "benchmark_log/devcluster/bench_batch_random_no_compression_results.csv"
# )
