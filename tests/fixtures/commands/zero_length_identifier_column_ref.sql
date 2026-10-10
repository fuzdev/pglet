-- The one position pglet was already loud in, and still miscoded: it resolved `""`
-- as a column name and answered `42703 column "" does not exist`, where 18.6 never
-- gets that far — the scanner refuses the token.
-- @setup
CREATE TABLE zli (a int);
-- @test
-- @expect-error: zero-length delimited identifier
SELECT "" FROM zli;
