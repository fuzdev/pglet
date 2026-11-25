-- `FROM ONLY t` is `t`: no table inherits from another here, so it has no descendants to
-- leave out.
-- @setup
CREATE TABLE fo (id INT);
INSERT INTO fo VALUES (1), (2);
-- @test
SELECT x.id FROM ONLY fo x WHERE x.id IN (SELECT id FROM ONLY fo) ORDER BY 1;
