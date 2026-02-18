# Configuration: Status

- current | absent | unsynced
- tracked file that is un-added



# TODO (editing needed)

    relative_path: relative path to the file with respect to where the operation was called

    status: (doesn’t include error status)

        current: the file is present in the project directory and matches the version in the storage directory

        absent: the file isn't present in the project directory

        unsynced: the file is present in the project directory, but doesn't match the version on in the storage directory

    file_size_bytes: current size of the file in bytes

    time_stamp: the ISO 8601 Zulu time of the most recent file version in the storage directory 

    saved_by: the user who uploaded the most recent file version in the storage directory

    message: the message inputted to the dvs_add command that added the most recent file version in the storage directory

    blake3_checksum: hash of the file via the blake3 algorithm

    absolute_path: canonicalized path of the file
  input:

    If inputted explicitly via file glob or path: the file name

    if inputted implicitly via dvs_status() (without input): NA

error: if the outcome was error, the error type, else NA

error message: if the outcome was error, the error message (if there was one), else NA
