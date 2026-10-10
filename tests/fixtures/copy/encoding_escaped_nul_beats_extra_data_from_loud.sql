-- Every field on the line is de-escaped and verified before the field count is checked,
-- so an escaped NUL outranks `extra data after last expected column`: `22021`, not
-- `22P04`.
-- @setup
CREATE TABLE ct (a text);
-- @copy-from
COPY ct FROM STDIN;
-- @data
x\000	y	z
-- @verify
SELECT a FROM ct;
