-- The row-bearing half of `enum_cast_text_param_zero_rows_accepts`: a row reaches the
-- conversion, and the bound label the enum does not hold is `22P02`.
-- @setup
CREATE TYPE cell_visibility AS ENUM ('private', 'public');
CREATE TABLE cell (id INT);
INSERT INTO cell VALUES (1);

-- @param $1 text secret
-- @test
-- @expect-error: secret is not a member of cell_visibility
-- @expect-message
SELECT ($1::text::cell_visibility)::text AS v FROM cell;
