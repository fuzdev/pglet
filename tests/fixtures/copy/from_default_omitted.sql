-- COPY ... FROM STDIN with an explicit column list that omits a DEFAULT column: the
-- omitted column takes its DEFAULT (Postgres + pglet alike). Verify the filled rows.
-- @setup
CREATE TABLE u (id int DEFAULT 99, name text, note text DEFAULT 'none');
-- @copy-from
COPY u (name, note) FROM STDIN;
-- @data
alice	hi
bob	\N
-- @verify
SELECT id, name, note FROM u ORDER BY name;
