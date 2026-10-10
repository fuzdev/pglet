-- `P` is a 1-based **Unicode codepoint** index: five two-byte characters advance it by
-- five, not ten. A byte offset would report 26 here where 18.6 reports 22.
-- @setup
CREATE TABLE t_sp2 (a INT);

-- @test
-- @expect-error: a trailing comma in the select list
SELECT 'ααααα' AS a, FROM t_sp2;
