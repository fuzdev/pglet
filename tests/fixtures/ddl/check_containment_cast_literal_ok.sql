-- The other half of `check_containment_uncast_literal_loud.sql`: with the literal
-- explicitly cast, a containment CHECK is accepted at DDL time and then actually
-- enforced on write — the point being that the DDL-time gate rejects only what would
-- have made the table unwritable, and does not reject a containment predicate as
-- such.
-- @setup
CREATE TABLE ck_cast (id INT, d JSONB, CHECK (d @> '{"ok":true}'::jsonb));
INSERT INTO ck_cast VALUES (1, '{"ok":true,"n":1}'), (2, '{"ok":true}');
-- @test
SELECT id, d FROM ck_cast ORDER BY id;
