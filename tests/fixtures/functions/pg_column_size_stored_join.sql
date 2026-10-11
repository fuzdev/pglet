-- A column of an aliased relation in a join is still read from that table's heap row.
-- @setup
CREATE TABLE pcj_a (id int, b bytea);
CREATE TABLE pcj_b (id int, t text);
INSERT INTO pcj_a VALUES (1, '\xdeadbeef'), (2, NULL);
INSERT INTO pcj_b VALUES (1, 'hello'), (2, 'x');
-- @test
SELECT a.id, pg_column_size(a.b) AS b, pg_column_size(x.t) AS t
FROM pcj_a a JOIN pcj_b x ON x.id = a.id ORDER BY a.id;
