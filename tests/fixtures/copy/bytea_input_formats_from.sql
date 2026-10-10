-- COPY text format hands each de-escaped field to the column's input function, so a
-- `bytea` field is read as a literal is: the hex format behind `\x`, the escape format
-- otherwise. COPY's own de-escaping runs first — `\\` in the data is one backslash.
-- @setup
CREATE TABLE t (id int, b bytea);
-- @copy-from
COPY t FROM STDIN;
-- @data
1	abcd
2	\\xabcd
3	\\x6162 63
4	\\\\
5	\\141b
6	 
7	\N
8	a\tb
9	hello
10	
\.
-- @verify
SELECT id, b FROM t ORDER BY id;
