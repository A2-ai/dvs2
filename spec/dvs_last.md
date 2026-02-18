# `dvs_last`

Goal: provide users with the ability to retrieve the result of
the last executed dvs command within the r package.

Example: Suppose after `dvs_add(by_folder = "data/derived/*")` was executed
an error occurred, and an overview is displayed as a data-frame. The user
got a R native result, a data-frame, but if the user wants to act on the
provided information, we might want to provide a `dvs_last` that contains
miscellaneous.
