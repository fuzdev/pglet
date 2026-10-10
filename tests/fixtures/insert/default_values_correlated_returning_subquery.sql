-- A correlated subquery in the `RETURNING` of a `DEFAULT VALUES` insert behaves
-- exactly as its `VALUES` twin does. The row source lowers to the same
-- `Plan::Insert` — one `InsertRows::Values` tuple, widened to table width with
-- `Expr::Default` — so `walk_level`'s `Insert` arm and `contains_subplan` need no new
-- arm; this fixture is the check on that claim rather than a new capability, and it
-- is red on an engine whose row-less source lowered to a shape the re-walk misses.

-- @setup
CREATE TABLE dv_corr_src (k TEXT, n INT);
INSERT INTO dv_corr_src VALUES ('x', 42), ('y', 7);
CREATE TABLE dv_corr (id SERIAL, v TEXT DEFAULT 'x');

-- @test
INSERT INTO dv_corr DEFAULT VALUES
  RETURNING id, v, (SELECT n FROM dv_corr_src o WHERE o.k = dv_corr.v) AS nn;
