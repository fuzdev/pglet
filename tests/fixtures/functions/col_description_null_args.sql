-- `col_description` is strict: a NULL in either place answers NULL, not an error.
-- @test
SELECT
  col_description(NULL, 1) AS null_oid,
  col_description(1, NULL) AS null_column,
  col_description(NULL, NULL) AS both_null;
