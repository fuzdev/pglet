-- The verification runs over the whole line before any field reaches its column's input
-- function, so an escaped NUL in the second field outranks a bad integer in the first:
-- `22021`, not `22P02`.
-- @setup
CREATE TABLE ct (a int, b text);
-- @copy-from
COPY ct FROM STDIN;
-- @data
notint	x\000
-- @verify
SELECT a, b FROM ct;
