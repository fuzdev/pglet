-- A NON-unique expression index over `LOWER(name)` WITH an opclass — the exact
-- `idx_actor_name_lower` shape (`LOWER(name) text_pattern_ops`). The opclass is
-- ignored; the index parses and loads with no enforcement, so two rows with the
-- same lowercased name both insert, and a plain select returns them.
-- @setup
CREATE TABLE expr_nonunique_lower (id INT, name TEXT);
CREATE INDEX ix_expr_nonunique_lower ON expr_nonunique_lower (LOWER(name) text_pattern_ops);
INSERT INTO expr_nonunique_lower VALUES (1, 'Alice'), (2, 'alice'), (3, 'Bob');

-- @test
SELECT id, name FROM expr_nonunique_lower ORDER BY id;
