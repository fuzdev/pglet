-- The conversion's own `22P02`, raised at plan time: over an **empty** table the
-- filter is never evaluated, so an eval-time-only conversion would answer zero
-- rows where 18.6 raises.
-- @setup
CREATE TABLE bcn6 (a INT);
-- @expect-error: invalid input syntax for type boolean
-- @test
SELECT count(*) FROM bcn6 WHERE 'abc';
