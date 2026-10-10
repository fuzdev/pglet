-- Precedence: `heap_create_with_catalog` raises the name collision before
-- `process_owned_by` resolves the owner, so a statement that is wrong both ways
-- reports 42P07 and not 42703.
-- @setup
CREATE TABLE own_prec_host (a INT);
CREATE SEQUENCE own_prec_seq;
-- @test
CREATE SEQUENCE own_prec_seq OWNED BY own_prec_host.nosuch;
-- @expect-error: relation already exists (42P07)
