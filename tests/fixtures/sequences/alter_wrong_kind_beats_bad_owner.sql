-- ...and the wrong-kind check likewise outranks the owner column (42809, not 42703).
-- @setup
CREATE TABLE awk_host (a INT);
-- @test
ALTER SEQUENCE awk_host OWNED BY awk_host.nosuch;
-- @expect-error: cannot open relation, not a sequence (42809)
