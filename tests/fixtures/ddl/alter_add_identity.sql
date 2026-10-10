-- `ALTER TABLE … ALTER COLUMN … ADD GENERATED … AS IDENTITY (…)` — the form
-- pg_dump emits for an identity column (not the inline CREATE TABLE form). The
-- `SEQUENCE NAME` / INCREMENT / MINVALUE / … options are accepted; only START WITH
-- is honored. After the ALTER the column auto-assigns 1, 2, ….
-- @setup
CREATE TABLE t (id int NOT NULL, name text);
ALTER TABLE t ALTER COLUMN id ADD GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME t_id_seq START WITH 1 INCREMENT BY 1 NO MINVALUE NO MAXVALUE CACHE 1);
INSERT INTO t (name) VALUES ('a'), ('b');
-- @test
SELECT id, name FROM t ORDER BY id;
