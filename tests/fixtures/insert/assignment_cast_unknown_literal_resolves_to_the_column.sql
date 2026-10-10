-- **A false-rejection control — do not trim it.** A bare string constant is Postgres's
-- `unknown`: it carries no type until the target column gives it one, so every cell here
-- is ordinary SQL on both engines. pglet's IR types such a literal as `text`, exactly as
-- it types the `'5'::text` spelling that 18.6 refuses, so the gate has to read the
-- expression's *shape* rather than its inferred type. Key it on `infer_type` instead and
-- every one of these columns becomes a spurious `42804`.
--
-- The `AS`-aliased spelling rides along because the alias is transparent on 18.6 too.
-- @setup
CREATE TABLE acu (id INT, b BOOL, i INT, ts TIMESTAMPTZ, u UUID, j JSONB);
-- @test
INSERT INTO acu (id, b, i, ts, u, j)
VALUES (1, 't', '5', '2024-03-01 10:00:00+00', '11111111-1111-1111-1111-111111111111', '{"k":1}')
RETURNING id, b, i, ts, u, j;
