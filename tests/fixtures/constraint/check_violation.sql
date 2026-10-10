-- @setup
CREATE TABLE ck (id INT, level TEXT CHECK (level IN ('viewer', 'editor')));

-- @test
-- @expect-error: value outside the CHECK set violates check constraint
INSERT INTO ck (id, level) VALUES (1, 'admin');
