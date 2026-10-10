-- The accept side of the same check: two escaped bytes spelling one valid character
-- (`\303\251` is `é`) decode to that character, as a raw `é` does.
-- @setup
CREATE TABLE ct (id int, a text);
-- @copy-from
COPY ct FROM STDIN;
-- @data
1	x\303\251y
2	xéy
-- @verify
SELECT id, a, length(a) FROM ct ORDER BY id;
