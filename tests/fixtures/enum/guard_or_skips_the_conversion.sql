-- The `OR` guard idiom: `s = ''` is `true` for the row, so `s::sce` is never evaluated and
-- the row is counted.
-- @setup
CREATE TYPE sce AS ENUM ('open', 'closed');
CREATE TABLE scg (id INT, st sce, s TEXT);
INSERT INTO scg VALUES (1, 'open', '');
-- @test
SELECT count(*) FROM scg WHERE (s = '' OR st = s::sce);
