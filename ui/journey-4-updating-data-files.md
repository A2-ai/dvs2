# Journey 4: Updating Data Files

Goal: Replace an existing tracked dataset with a new version.

## CLI flow
1. Re-run your processing to overwrite the data file
   ```bash
   # Your data pipeline updates:
   # data/derived/pk_data.csv
   ```
2. Check status
   ```bash
   dvs status data/derived/pk_data.csv
   ```
3. Add the new version
   ```bash
   dvs add data/derived/pk_data.csv --message "Updated PK dataset v2"
   ```
4. Commit updated metadata
   ```bash
   git add data/derived/pk_data.csv.dvs
   git commit -m "Update PK data with new processing"
   git push
   ```

## R package flow
1. Re-run your processing
   ```r
   pk_data_v2 <- update_processing(pk_data)
   write.csv(pk_data_v2, "data/derived/pk_data.csv")
   ```
2. Check status
   ```r
   dvs_status("data/derived/pk_data.csv")
   ```
3. Add the new version
   ```r
   dvs_add("data/derived/pk_data.csv", message = "Updated PK dataset v2")
   ```
4. Commit updated metadata
   ```bash
   git add data/derived/pk_data.csv.dvs
   git commit -m "Update PK data with new processing"
   git push
   ```
