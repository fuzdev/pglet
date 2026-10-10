-- The write polarity of `enum/from_derived_table_computed_column_accepts`: an
-- `UPDATE … FROM` binds `v.z` through a namespace built by a *different* route
-- (`push_plan_relations`, not `build_column_scope`), so a fix that reached only the
-- query route leaves this one refusing a write 18.6 performs.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL, n INT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 0), (2, 'ok', 0), (3, 'happy', 0);
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
UPDATE w SET n = 9 FROM (SELECT min(m) AS z FROM u) v
WHERE w.m >= v.z RETURNING w.id, w.m::text AS m_text, w.n;
