-- Having derived the identity, the comparison must order by the enum's **member
-- positions** and not by its labels' bytes: `ok < happy` by position, `happy < ok` by
-- bytes, so the row set separates the two rules.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL);
INSERT INTO w VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
SELECT w.id FROM w, (SELECT min(m) AS z FROM u) v WHERE w.m > v.z ORDER BY w.id;
