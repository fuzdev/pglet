-- `DEFAULT VALUES` composes with `RETURNING`, wildcard included — `insert_rest`'s
-- `DEFAULT VALUES` alternative sits under the same `opt_on_conflict returning_clause`
-- tail every other row source does. The extended-protocol leg is the point as much as
-- the values: `Describe` must answer the three target columns for a statement whose
-- row source has no cells to infer from.

-- @setup
CREATE TABLE dv_ret (id SERIAL, v TEXT DEFAULT 'x', n INT);

-- @test
INSERT INTO dv_ret DEFAULT VALUES RETURNING *;
