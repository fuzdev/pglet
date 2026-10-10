-- A value inside the CHECK set inserts cleanly.
-- @setup
CREATE TABLE ck (id INT, level TEXT CHECK (level IN ('viewer', 'editor')));
INSERT INTO ck (id, level) VALUES (1, 'viewer'), (2, 'editor');

-- @test
SELECT id, level FROM ck ORDER BY id;
