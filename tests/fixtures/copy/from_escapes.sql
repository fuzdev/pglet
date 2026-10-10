-- COPY ... FROM STDIN must decode the COPY text escapes: \t -> tab, \n -> newline,
-- \\ -> backslash inside a field (so an embedded delimiter/newline survives the
-- round-trip). Verify the decoded values match Postgres.
-- @setup
CREATE TABLE w (id int, body text);
-- @copy-from
COPY w FROM STDIN;
-- @data
1	has\ttab
2	has\nnewline
3	has\\backslash
4	all\t\n\\three
-- @verify
SELECT id, body FROM w ORDER BY id;
