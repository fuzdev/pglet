-- The accept-side pin for the index decline: an *equality* over an enum column answers
-- the same rows whether or not the index serves it, because one member is one label is
-- one OID and 18.6's `enum_eq` is bare OID identity. Declining the index costs this
-- query speed and nothing else, and a fix that moved enum **equality** to member
-- position (a second computation of the same answer) reddens nothing here — which is
-- the point: this fixture pins that the answer did not move.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE INDEX t_m_idx ON t (m);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy'), (4, 'ok');

-- @test
SELECT id FROM t WHERE m = 'ok' ORDER BY id;
