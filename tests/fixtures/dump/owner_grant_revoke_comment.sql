-- The role/comment statements pg_dump emits have no semantic effect in pglet
-- (no roles, privileges, or object comments). They must parse and succeed as
-- no-ops, leaving the data untouched.
-- @setup
CREATE TABLE dpriv (id int, name text);
INSERT INTO dpriv VALUES (1, 'a'), (2, 'b');
ALTER TABLE dpriv OWNER TO postgres;
ALTER TABLE ONLY dpriv OWNER TO postgres;
GRANT ALL ON TABLE dpriv TO postgres;
REVOKE ALL ON TABLE public.dpriv FROM postgres;
COMMENT ON TABLE dpriv IS 'a comment';
COMMENT ON COLUMN dpriv.name IS 'the name column';
-- @test
SELECT id, name FROM dpriv ORDER BY id;
