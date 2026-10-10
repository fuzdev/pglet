-- The bound guard idiom with an empty string: `$1 = ''` fixes `$1` as `text` and is `true`,
-- so `$1::sce` converts only where it is evaluated — and it is not.
-- @setup
CREATE TYPE sce AS ENUM ('open', 'closed');
CREATE TABLE scg (id INT, st sce);
INSERT INTO scg VALUES (1, 'open');
-- @param $1 text ''
-- @test
SELECT count(*) FROM scg WHERE ($1 = '' OR st = $1::sce);
