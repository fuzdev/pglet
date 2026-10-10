-- The guard idiom: `s <> ''` is `false` for the row, so `s::sce` — a conversion of the
-- empty string, which the enum does not hold — is never evaluated and no row is counted.
-- @setup
CREATE TYPE sce AS ENUM ('open', 'closed');
CREATE TABLE scg (id INT, st sce, s TEXT);
INSERT INTO scg VALUES (1, 'open', '');
-- @test
SELECT count(*) FROM scg WHERE (s <> '' AND st = s::sce);
