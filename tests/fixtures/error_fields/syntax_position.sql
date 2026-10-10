-- 42601 carries `P` — and pglet used to spell a 0-based **byte** offset into the
-- message instead, where 18.6's message never carries a position at all. The message
-- itself still diverges (pglet names what it expected), so only `P` is compared here.
-- @setup
CREATE TABLE t_sp (a INT);

-- @test
-- @expect-error: a WHERE clause with no predicate
SELECT * FROM t_sp WHERE;
