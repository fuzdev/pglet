-- A `DO $$ … $$` idempotent CREATE TYPE block — the fuz_app cell migration idiom.
-- `CREATE TYPE` has no `IF NOT EXISTS`, so the block swallows `duplicate_object` on
-- replay. Run twice in setup (proving idempotency), then prove the enum type is
-- created and usable as a column type with its members. The enum column is read
-- back as `::text` — a custom enum's dynamic OID isn't decodable by the binary
-- harness driver (tokio-postgres), so the text cast keeps the comparison stable
-- while still proving the stored members (`'private'` default, `'public'`).
-- @setup
DO $$ BEGIN
    CREATE TYPE cell_visibility AS ENUM ('private', 'public');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;
DO $$ BEGIN
    CREATE TYPE cell_visibility AS ENUM ('private', 'public');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;
CREATE TABLE cell (id INT, visibility cell_visibility NOT NULL DEFAULT 'private');
INSERT INTO cell (id) VALUES (1);
INSERT INTO cell (id, visibility) VALUES (2, 'public');
-- @test
SELECT id, visibility::text AS visibility FROM cell ORDER BY id;
