# Journey 2: Adding Data Files

Goal: Version a newly created dataset so others can retrieve it.

## CLI flow
1. Produce the data (example)
   ```bash
   # Your data pipeline or script writes:
   # data/derived/pk_data.csv
   ```
2. Add the file to DVS
   ```bash
   dvs add data/derived/pk_data.csv --message "Initial PK dataset v1"
   ```
3. Commit DVS metadata
   ```bash
   git add data/derived/pk_data.csv.dvs data/derived/.gitignore
   git commit -m "Add processed PK data"
   git push
   ```
4. Verify status
   ```bash
   dvs status data/derived/pk_data.csv
   ```

## R package flow
1. Produce the data
   ```r
   write.csv(pk_data, "data/derived/pk_data.csv")
   ```
2. Add the file to DVS
   ```r
   dvs_add("data/derived/pk_data.csv", message = "Initial PK dataset v1")
   ```
3. Commit DVS metadata
   ```bash
   git add data/derived/pk_data.csv.dvs data/derived/.gitignore
   git commit -m "Add processed PK data"
   git push
   ```
4. Verify status
   ```r
   dvs_status("data/derived/pk_data.csv")
   ```
