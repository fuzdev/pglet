-- The write the boundary reached. A `DELETE` whose `WHERE` is a bare scalar subquery
-- compared against a member is ordinary SQL both engines accept, and under the label
-- bytes it removed every row where 18.6 removes none — no error anywhere.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES (1, 'happy');
DELETE FROM t WHERE (SELECT m FROM u WHERE id = 1) < 'ok';

-- @test
SELECT id FROM t ORDER BY id;
