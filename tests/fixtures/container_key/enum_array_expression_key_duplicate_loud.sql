-- The equality half of an enum-array expression key: one row per member.
-- @profile-lock
-- @setup
CREATE TYPE cku_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE cku_m (id INT, m cku_mood);
CREATE UNIQUE INDEX cku_m_u ON cku_m ((ARRAY[m]));
INSERT INTO cku_m (m, id) VALUES ('happy', 1), ('sad', 2), ('ok', 3);
-- @test
-- @expect-error: a duplicate enum-array expression key (23505)
INSERT INTO cku_m (m, id) VALUES ('sad', 4);
