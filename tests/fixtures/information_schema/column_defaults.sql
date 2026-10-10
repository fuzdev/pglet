-- column_default renders the catalog's stored DEFAULT to Postgres's spelling:
-- gen_random_uuid() / now() / false / 1 / 'success'::text /
-- nextval('<seq>'::regclass). The ORDER BY is on the non-projected
-- ordinal_position, so this also exercises the Sort-below-projection path over a
-- virtual scan.
-- @setup
CREATE TABLE cd_t (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  flag BOOL NOT NULL DEFAULT false,
  n INT DEFAULT 1,
  status TEXT DEFAULT 'success',
  seq BIGSERIAL
);

-- @test
SELECT column_name::text AS column_name, column_default::text AS column_default
FROM information_schema.columns
WHERE table_schema = 'public' AND table_name = 'cd_t' AND column_default IS NOT NULL
ORDER BY ordinal_position;
