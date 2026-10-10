-- A foreign key between two columns of one type is accepted for every type, the polymorphic
-- classes included: the same enum, the same array type, `jsonb`, `uuid`, `boolean`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok');
CREATE TABLE fpm (k mood PRIMARY KEY);
CREATE TABLE fpa (k INT[] PRIMARY KEY);
CREATE TABLE fpta (k TEXT[] PRIMARY KEY);
CREATE TABLE fpj (k JSONB PRIMARY KEY);
CREATE TABLE fpu (k UUID PRIMARY KEY);
CREATE TABLE fpb (k BOOL PRIMARY KEY);
CREATE TABLE fcm (k mood REFERENCES fpm);
CREATE TABLE fca (k INT[] REFERENCES fpa);
CREATE TABLE fcta (k TEXT[] REFERENCES fpta);
CREATE TABLE fcj (k JSONB REFERENCES fpj);
CREATE TABLE fcu (k UUID REFERENCES fpu);
CREATE TABLE fcb (k BOOL REFERENCES fpb);

-- @test
SELECT conname, contype FROM pg_constraint WHERE contype = 'f' ORDER BY conname;
