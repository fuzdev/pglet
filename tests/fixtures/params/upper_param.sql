-- A bound text param as the argument of a string function (upper / split_part). The
-- param must type as text from its argument position; left untyped (OID 0) a
-- binary-protocol client (tokio-postgres) loops on type introspection. Exercises the
-- extended-protocol binding path.
-- @setup
CREATE TABLE doc (id INT, name TEXT);
INSERT INTO doc VALUES (1, 'alpha'), (2, 'beta');
-- @param $1 text alpha
-- @test
SELECT id, upper($1) AS up, split_part($1, 'a', 1) AS part FROM doc WHERE name = $1;
