-- Verify the synthesized rows match Postgres byte-for-byte, including the
-- TypeOid -> data_type name mapping. The projected columns are cast to plain
-- text/int so Postgres's information_schema domains (sql_identifier,
-- character_data, cardinal_number) decode cleanly; on pglet the casts are no-ops.
-- @setup
CREATE TABLE isc_t (id INT NOT NULL, name TEXT, tags TEXT[], created TIMESTAMPTZ, flag BOOL, n BIGINT);

-- @test
SELECT
  column_name::text AS column_name,
  data_type::text AS data_type,
  is_nullable::text AS is_nullable,
  ordinal_position::int AS ordinal_position
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'isc_t'
ORDER BY ordinal_position;
