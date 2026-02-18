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

## Journey 5: Updating data files with new rows

New data following previous form might come up. Example is new rows from a clinical trial,
new participants in trials is added, however the scientists want them added to already
checked data files.

```r
dvs_add("data/registry/participants.csv", "added information from the second batch of runs")
```

this ought to say

```r
> "Error: file already exists; consider noting if this is an amendment to the previous file via `amend = TRUE`"
```

Then,

```r
dvs_add("data/registry/participants.csv", "added information from the second batch of runs", amend = TRUE)
```

could be executed, in which: Previous hash is compared to the new file `data/registry/participants.csv`, but truncated
to the level of the previous file, and then it can be known if this new event can supersede other add events, because we
know it is an addition.

The hash itself cannot distinguish between a completely new file, or one with new bytes. In dvs, we only have current hash,
so we should consider adding this context via the user, i.e. by asking if it is an addition / amendment.
