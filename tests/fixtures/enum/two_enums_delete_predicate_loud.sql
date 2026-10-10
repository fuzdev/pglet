-- The write route: `DELETE FROM v WHERE m > y` was `DELETE 1` here and destroyed the row
-- 18.6 keeps by refusing.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE v (id INT, m mood NOT NULL, y m3 NOT NULL);
INSERT INTO v VALUES (1, 'ok', 'sad'), (2, 'ok', 'happy'), (3, 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood > m3
-- @expect-message
DELETE FROM v WHERE m > y;
