-- The other field-count verdict: a line with too few fields whose one field holds an
-- escaped NUL is `22021`, not `22P04 missing data for column`, because the fields are
-- de-escaped and verified before the count is checked.
-- @setup
CREATE TABLE ct (a int, b text);
-- @copy-from
COPY ct FROM STDIN;
-- @data
x\000
-- @verify
SELECT a, b FROM ct;
