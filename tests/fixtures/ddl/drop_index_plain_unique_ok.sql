-- A standalone CREATE UNIQUE INDEX carries no constraint marker, so DROP INDEX
-- takes it — Postgres's split between an index and a constraint's index. Once it is
-- gone the duplicate insert is accepted on both engines.
-- @setup
CREATE TABLE dipu (id INT PRIMARY KEY, v INT);
CREATE UNIQUE INDEX dipu_v_ux ON dipu (v);

-- @profile-lock
-- @test
DROP INDEX dipu_v_ux;
INSERT INTO dipu VALUES (1, 7), (2, 7);
SELECT id, v FROM dipu ORDER BY id;
