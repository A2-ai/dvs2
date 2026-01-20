# Journey 5: Working with Multiple Files

Goal: Add and retrieve batches of outputs with glob patterns.

## CLI flow
1. Produce multiple outputs
   ```bash
   # Your data pipeline writes:
   # data/derived/pk.csv
   # data/derived/pd.csv
   # data/derived/summary.csv
   ```
2. Add all outputs at once
   ```bash
   dvs add data/derived/*.csv --message "Analysis outputs batch 1"
   ```
3. Retrieve all tracked files later
   ```bash
   dvs get data/derived/*.csv
   ```
4. Check status for everything
   ```bash
   dvs status
   ```

## R package flow
1. Produce multiple outputs
   ```r
   write.csv(pk_data, "data/derived/pk.csv")
   write.csv(pd_data, "data/derived/pd.csv")
   write.csv(summary_stats, "data/derived/summary.csv")
   ```
2. Add all outputs at once
   ```r
   dvs_add("data/derived/*.csv", message = "Analysis outputs batch 1")
   ```
3. Retrieve all tracked files later
   ```r
   dvs_get("data/derived/*.csv")
   ```
4. Check status for everything
   ```r
   dvs_status()
   ```
