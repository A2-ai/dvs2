# Evaluate `call` and check that it raises a dvs_error_argument for `param`,
# naming `call` itself, the call the user wrote.
expect_argument_error <- function(call, param) {
  cnd <- tryCatch(eval(call, parent.frame()), error = identity)
  expect_s3_class(cnd, "dvs_error_argument")
  expect_s3_class(cnd, "dvs_error")
  expect_equal(cnd$param, param)
  expect_identical(conditionCall(cnd), call)
}

# The code of a failed command, from a condition that must be a dvs_error.
error_code <- function(expr) {
  cnd <- tryCatch(expr, error = identity)
  expect_s3_class(cnd, "dvs_error")
  cnd$code
}
