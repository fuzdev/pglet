/**
 * Kysely type definitions for pglet compatibility testing.
 *
 * Mirrors the Drizzle schema for consistent test coverage.
 */

import type { Generated, Insertable, Selectable, Updateable } from "kysely";

export interface Database {
  users: UsersTable;
  posts: PostsTable;
}

export interface UsersTable {
  id: number;
  name: string;
  email: string;
  active: boolean | null;
  created_at: Date | null;
}

export interface PostsTable {
  id: number;
  title: string;
  content: string | null;
  author_id: number;
  published: boolean | null;
}

// Convenience types for CRUD operations
export type User = Selectable<UsersTable>;
export type NewUser = Insertable<UsersTable>;
export type UserUpdate = Updateable<UsersTable>;

export type Post = Selectable<PostsTable>;
export type NewPost = Insertable<PostsTable>;
export type PostUpdate = Updateable<PostsTable>;
