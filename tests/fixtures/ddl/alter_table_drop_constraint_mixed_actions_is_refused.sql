-- Documentary: a list mixing `DROP CONSTRAINT` with another action is pglet's `0A000`,
-- where 18.6 performs both. Any pglet error passes.
-- @expect-error: pglet performs a list only when every action is DROP CONSTRAINT
-- @setup
CREATE TABLE dcmx (a INT CHECK (a > 0));

-- @test
ALTER TABLE dcmx DROP CONSTRAINT dcmx_a_check, ADD COLUMN b INT;
