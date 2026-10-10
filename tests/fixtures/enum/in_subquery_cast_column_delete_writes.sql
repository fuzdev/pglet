-- The accept polarity **writes**, which is what makes the over-refusal above worth a
-- fixture of its own: the `DELETE` below removes the row 18.6 removes. The survivors are
-- the comparison.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

DELETE FROM w WHERE m IN (SELECT 'ok'::mood);

-- @test
SELECT id, m::text, s FROM w ORDER BY id;
