-- pg_dump emits a SERIAL column as: a plain integer column, an explicit
-- CREATE SEQUENCE (with the full option set), ALTER SEQUENCE OWNED BY, then
-- ALTER COLUMN SET DEFAULT nextval(...). pglet must accept all of it and the
-- re-attached default must produce the sequence's values.
-- @setup
CREATE TABLE dseq (id integer NOT NULL, name text);
CREATE SEQUENCE dseq_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;
ALTER SEQUENCE dseq_id_seq OWNED BY dseq.id;
ALTER TABLE ONLY dseq ALTER COLUMN id SET DEFAULT nextval('dseq_id_seq');
INSERT INTO dseq (name) VALUES ('first');
INSERT INTO dseq (name) VALUES ('second');
-- @test
SELECT id, name FROM dseq ORDER BY id;
