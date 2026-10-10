-- The real `idx_account_email` shape: a PARTIAL expression unique index over
-- `LOWER(email) WHERE email IS NOT NULL`. Rows with NULL email fail the predicate
-- (and have a NULL key), so any number of them coexist; only the non-NULL rows
-- participate in the LOWER-key uniqueness. Here three NULL-email rows plus two
-- distinct-LOWER non-NULL rows all insert.
-- @setup
CREATE TABLE expr_lower_partial (id INT, email TEXT);
CREATE UNIQUE INDEX ux_expr_lower_partial
  ON expr_lower_partial (LOWER(email)) WHERE email IS NOT NULL;
INSERT INTO expr_lower_partial VALUES
  (1, NULL), (2, NULL), (3, NULL), (4, 'a@x'), (5, 'B@x');

-- @test
SELECT id, email FROM expr_lower_partial ORDER BY id;
