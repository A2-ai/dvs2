test_that("dvs_bytes preserves integer storage from the dvs library", {
  x <- dvs:::new_dvs_bytes(c(100L, 2048L, 5000000L))
  expect_s3_class(x, "dvs_bytes")
  expect_equal(typeof(unclass(x)), "integer")
  expect_equal(unclass(x), c(100L, 2048L, 5000000L))
})

test_that("dvs_bytes coerces Option<u64> list(NULL) dry_run values to NA_integer_", {
  x <- dvs:::new_dvs_bytes(list(NULL, 100L, NULL))
  expect_equal(typeof(unclass(x)), "integer")
  expect_equal(unclass(x), c(NA_integer_, 100L, NA_integer_))
})

test_that("dvs_bytes supports addition and subtraction, preserving class", {
  x <- dvs:::new_dvs_bytes(c(1024L, 2048L))
  y <- dvs:::new_dvs_bytes(c(100L, 200L))

  sum_xy <- x + y
  diff_xy <- x - y

  expect_s3_class(sum_xy, "dvs_bytes")
  expect_s3_class(diff_xy, "dvs_bytes")
  expect_equal(unclass(sum_xy), c(1124L, 2248L))
  expect_equal(unclass(diff_xy), c(924L, 1848L))
})

test_that("dvs_bytes arithmetic with a plain numeric/integer preserves class", {
  x <- dvs:::new_dvs_bytes(c(1024L, 2048L))
  plus_scalar <- x + 10L
  minus_scalar <- x - 10L

  expect_s3_class(plus_scalar, "dvs_bytes")
  expect_s3_class(minus_scalar, "dvs_bytes")
  expect_equal(unclass(plus_scalar), c(1034L, 2058L))
  expect_equal(unclass(minus_scalar), c(1014L, 2038L))
})

test_that("dvs_bytes comparison operators return logical (not dvs_bytes)", {
  x <- dvs:::new_dvs_bytes(c(100L, 200L, 300L))
  result <- x > 150L
  expect_type(result, "logical")
  expect_equal(result, c(FALSE, TRUE, TRUE))
})

test_that("sum() over dvs_bytes preserves class", {
  x <- dvs:::new_dvs_bytes(c(1024L, 2048L, 512L))
  total <- sum(x)
  expect_s3_class(total, "dvs_bytes")
  expect_equal(unclass(total), 3584L)
})

test_that("min() and max() over dvs_bytes preserve class", {
  x <- dvs:::new_dvs_bytes(c(1024L, 2048L, 512L))
  expect_s3_class(min(x), "dvs_bytes")
  expect_s3_class(max(x), "dvs_bytes")
  expect_equal(unclass(min(x)), 512L)
  expect_equal(unclass(max(x)), 2048L)
})

test_that("pillar_shaft.dvs_bytes formats bytes through format_byte_size", {
  x <- dvs:::new_dvs_bytes(c(100L, 2048L, 5000000L))
  shaft <- pillar::pillar_shaft(x)
  expect_s3_class(shaft, "pillar_shaft")
  formatted <- format(shaft, width = pillar::get_max_extent(shaft))
  out <- as.character(formatted)
  expect_match(out[1], "B$")
  expect_match(out[2], "KB$")
  expect_match(out[3], "MB$")
})

test_that("pillar_shaft.dvs_bytes renders NA_integer_ as NA", {
  x <- dvs:::new_dvs_bytes(c(100L, NA_integer_))
  shaft <- pillar::pillar_shaft(x)
  formatted <- format(shaft, width = pillar::get_max_extent(shaft))
  out <- as.character(formatted)
  expect_true(any(grepl("NA", out)))
})

test_that("type_sum.dvs_bytes returns 'bytes'", {
  x <- dvs:::new_dvs_bytes(100L)
  expect_equal(pillar::type_sum(x), "bytes")
})

test_that("tibble rendering shows 'bytes' type and human-readable sizes", {
  tbl <- tibble::tibble(
    file = c("a", "b"),
    size = dvs:::new_dvs_bytes(c(1024L, 1048576L))
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
