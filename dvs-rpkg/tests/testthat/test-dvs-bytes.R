test_that("new_dvs_bytes stores as double so >2 GB sizes don't overflow", {
  x <- new_dvs_bytes(c(100, 2048, 5e6))
  expect_s3_class(x, "dvs_bytes")
  expect_equal(typeof(unclass(x)), "double")
  expect_equal(unclass(x), c(100, 2048, 5e6))

  big <- new_dvs_bytes(4e9)  # 4 GB, would overflow 32-bit integer
  expect_equal(unclass(big), 4e9)
  expect_false(is.na(unclass(big)))
})

test_that("new_dvs_bytes handles NA, NULL and zero-length input", {
  expect_equal(length(new_dvs_bytes(NULL)), 0L)
  expect_s3_class(new_dvs_bytes(NULL), "dvs_bytes")
  expect_equal(length(new_dvs_bytes(integer(0))), 0L)
  expect_true(is.na(unclass(new_dvs_bytes(NA_real_))))
})

test_that("new_dvs_bytes and format_byte_size handle 2.5 GB without overflow", {
  size_bytes <- 2.5 * 1024^3  # 2,684,354,560 — exceeds 32-bit integer max

  # R-side: new_dvs_bytes must store it as double without precision loss
  x <- new_dvs_bytes(size_bytes)
  expect_s3_class(x, "dvs_bytes")
  expect_equal(typeof(unclass(x)), "double")
  expect_equal(unclass(x), size_bytes)
  expect_false(is.na(unclass(x)))

  # Rust-side: format_byte_size receives the u64 and must format it in GB
  formatted <- format_byte_size(size_bytes)
  expect_match(formatted, "GB")
  expect_match(formatted, "2\\.5")
})

test_that("dvs_bytes + and - preserve class", {
  x <- new_dvs_bytes(c(1024, 2048))
  y <- new_dvs_bytes(c(100, 200))

  sum_xy <- x + y
  diff_xy <- x - y

  expect_s3_class(sum_xy, "dvs_bytes")
  expect_s3_class(diff_xy, "dvs_bytes")
  expect_equal(unclass(sum_xy), c(1124, 2248))
  expect_equal(unclass(diff_xy), c(924, 1848))
})

test_that("dvs_bytes + scalar preserves class", {
  x <- new_dvs_bytes(c(1024, 2048))
  expect_s3_class(x + 10, "dvs_bytes")
  expect_s3_class(x - 10, "dvs_bytes")
  expect_equal(unclass(x + 10), c(1034, 2058))
})

test_that("dvs_bytes * and / do NOT reclass (bytes*bytes != bytes)", {
  x <- new_dvs_bytes(c(1024, 2048))
  expect_false(inherits(x * x, "dvs_bytes"))
  expect_false(inherits(x / 2, "dvs_bytes"))
  expect_equal(x * 2, c(2048, 4096))
})

test_that("dvs_bytes comparisons return plain logicals", {
  x <- new_dvs_bytes(c(100, 200, 300))
  expect_type(x > 150, "logical")
  expect_equal(x > 150, c(FALSE, TRUE, TRUE))
  expect_equal(x == x, c(TRUE, TRUE, TRUE))
})

test_that("sum/min/max/range over dvs_bytes preserve class", {
  x <- new_dvs_bytes(c(1024, 2048, 512))
  expect_s3_class(sum(x), "dvs_bytes")
  expect_s3_class(min(x), "dvs_bytes")
  expect_s3_class(max(x), "dvs_bytes")
  expect_s3_class(range(x), "dvs_bytes")
  expect_equal(unclass(sum(x)), 3584)
  expect_equal(unclass(range(x)), c(512, 2048))
})

test_that("dvs_add columns size + stored_size yield a dvs_bytes column", {
  new_dvs_test_repo()
  write_theoph("t.csv")
  r <- dvs_add(paths = file.path(getwd(), "t.csv"))

  total <- r$size + r$stored_size
  expect_s3_class(total, "dvs_bytes")
  expect_equal(typeof(unclass(total)), "double")
})

test_that("pillar_shaft.dvs_bytes formats bytes via format_byte_size", {
  x <- new_dvs_bytes(c(100, 2048, 5000000))
  shaft <- pillar::pillar_shaft(x)
  expect_s3_class(shaft, "pillar_shaft")
  out <- as.character(format(shaft, width = pillar::get_max_extent(shaft)))
  expect_match(out[1], "B$")
  expect_match(out[2], "KB$")
  expect_match(out[3], "MB$")
})

test_that("pillar_shaft.dvs_bytes on empty input returns empty output", {
  x <- new_dvs_bytes(numeric(0))
  shaft <- pillar::pillar_shaft(x)
  expect_s3_class(shaft, "pillar_shaft")
})

test_that("pillar_shaft.dvs_bytes renders NA as NA", {
  x <- new_dvs_bytes(c(100, NA_real_))
  shaft <- pillar::pillar_shaft(x)
  out <- as.character(format(shaft, width = pillar::get_max_extent(shaft)))
  expect_true(any(grepl("NA", out)))
})

test_that("type_sum.dvs_bytes returns 'bytes'", {
  expect_equal(pillar::type_sum(new_dvs_bytes(100)), "bytes")
})

test_that("tibble print shows 'bytes' type and human-readable sizes", {
  tbl <- tibble::tibble(
    file = c("a", "b"),
    size = new_dvs_bytes(c(1024, 1048576))
  )
  out <- capture.output(print(tbl))
  expect_true(any(grepl("bytes", out)))
  expect_true(any(grepl("KB|MB", out)))
})

test_that("format_byte_size returns a human-readable string", {
  expect_match(format_byte_size(0), "B")
  expect_match(format_byte_size(1024), "KB")
  expect_match(format_byte_size(1048576), "MB")
})
