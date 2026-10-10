-- `CREATE SEQUENCE … AS <type>` never resolves a type name against the catalog — it
-- reads a kind — so this position agrees only because the refusal is a **parse-time**
-- one on both engines. The in-range `AS FLOAT(1)` spelling still parts here
-- (`22023 sequence type must be smallint, integer, or bigint` there, accepted here):
-- that is the position's own gap, recorded in `EDGES.md`, not this refusal's.
-- @expect-error: a float precision below 1 bit at a sequence type
-- @expect-message
-- @test
CREATE SEQUENCE zsq AS FLOAT(0);
