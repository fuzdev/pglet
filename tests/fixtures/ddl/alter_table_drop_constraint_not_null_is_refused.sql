-- Documentary: dropping a `NOT NULL` constraint by its `<table>_<column>_not_null` name
-- is pglet's `0A000`, a loud refusal where 18.6 makes the column nullable. 18.6
-- accepts, so any pglet error passes.
-- @expect-error: pglet does not drop a not-null constraint
-- @setup
CREATE TABLE dcnn (a INT NOT NULL);

-- @test
ALTER TABLE dcnn DROP CONSTRAINT dcnn_a_not_null;
