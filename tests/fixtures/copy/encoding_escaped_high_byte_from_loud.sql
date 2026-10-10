-- An escaped byte that starts no legal UTF-8 sequence (`\xe9` followed by `y`) is the same
-- `22021` as a NUL, not a COPY-format `22P04`.
-- @setup
CREATE TABLE ct (a text);
-- @copy-from
COPY ct FROM STDIN;
-- @data
x\xe9y
-- @verify
SELECT a FROM ct;
