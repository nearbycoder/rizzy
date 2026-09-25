PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY,
  external_account_id INTEGER NOT NULL UNIQUE,
  name TEXT NOT NULL,
  cards_count INTEGER NOT NULL DEFAULT 0,
  auto_postpone_period_in_days INTEGER NOT NULL DEFAULT 30,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS account_join_codes (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL UNIQUE REFERENCES accounts(id) ON DELETE CASCADE,
  code TEXT NOT NULL UNIQUE,
  usage_count INTEGER NOT NULL DEFAULT 0,
  usage_limit INTEGER NOT NULL DEFAULT 10,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS identities (
  id TEXT PRIMARY KEY,
  email_address TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  identity_id TEXT NOT NULL REFERENCES identities(id),
  name TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'member',
  active INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  UNIQUE(account_id, identity_id)
);
CREATE UNIQUE INDEX IF NOT EXISTS users_system_account ON users(account_id) WHERE role='system';
CREATE TABLE IF NOT EXISTS access_tokens (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id),
  token_hash TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL,
  permission TEXT NOT NULL CHECK(permission IN ('read','write')),
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS magic_links (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  code TEXT NOT NULL UNIQUE,
  purpose TEXT NOT NULL CHECK(purpose IN ('sign_in','sign_up')),
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS magic_links_expiration ON magic_links(expires_at);
CREATE TABLE IF NOT EXISTS pending_authentications (
  token_hash TEXT PRIMARY KEY,
  email_address TEXT NOT NULL,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS pending_authentications_expiration ON pending_authentications(expires_at);
CREATE TABLE IF NOT EXISTS sessions (
  id TEXT PRIMARY KEY,
  identity_id TEXT NOT NULL REFERENCES identities(id) ON DELETE CASCADE,
  token_hash TEXT NOT NULL UNIQUE,
  user_agent TEXT,
  signup_pending INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS sessions_identity ON sessions(identity_id);
CREATE TABLE IF NOT EXISTS authentication_attempts (
  id TEXT PRIMARY KEY,
  bucket TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS authentication_attempts_bucket ON authentication_attempts(bucket,created_at);
CREATE INDEX IF NOT EXISTS authentication_attempts_age ON authentication_attempts(created_at);
CREATE TABLE IF NOT EXISTS boards (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  creator_id TEXT NOT NULL REFERENCES users(id),
  name TEXT NOT NULL,
  all_access INTEGER NOT NULL DEFAULT 1,
  auto_postpone_period_in_days INTEGER NOT NULL DEFAULT 30,
  entropy_override INTEGER NOT NULL DEFAULT 0,
  public_description TEXT NOT NULL DEFAULT '',
  publication_key TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS boards_account ON boards(account_id, created_at DESC);
CREATE TABLE IF NOT EXISTS accesses (
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  involvement TEXT NOT NULL DEFAULT 'access_only' CHECK(involvement IN ('access_only','watching')),
  PRIMARY KEY(board_id, user_id)
);
CREATE INDEX IF NOT EXISTS accesses_user_board ON accesses(user_id,board_id);
CREATE TABLE IF NOT EXISTS columns (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  color TEXT NOT NULL DEFAULT 'var(--color-card-default)',
  position INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS columns_board_position ON columns(board_id, position);
CREATE TABLE IF NOT EXISTS cards (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  column_id TEXT REFERENCES columns(id) ON DELETE SET NULL,
  creator_id TEXT NOT NULL REFERENCES users(id),
  number INTEGER NOT NULL,
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'published',
  closed INTEGER NOT NULL DEFAULT 0,
  postponed INTEGER NOT NULL DEFAULT 0,
  golden INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_active_at TEXT NOT NULL,
  UNIQUE(account_id, number)
);
CREATE INDEX IF NOT EXISTS cards_board_active ON cards(board_id, status, closed, postponed, last_active_at DESC);
CREATE INDEX IF NOT EXISTS cards_account_latest ON cards(account_id, last_active_at DESC);
CREATE TABLE IF NOT EXISTS steps (
  id TEXT PRIMARY KEY,
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  content TEXT NOT NULL,
  completed INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS comments (
  id TEXT PRIMARY KEY,
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  creator_id TEXT NOT NULL REFERENCES users(id),
  body TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT
);
CREATE TABLE IF NOT EXISTS tags (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  title TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE(account_id,title)
);
CREATE TABLE IF NOT EXISTS taggings (
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
  PRIMARY KEY(card_id,tag_id)
);
CREATE TABLE IF NOT EXISTS assignments (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  assignee_id TEXT NOT NULL REFERENCES users(id),
  assigner_id TEXT NOT NULL REFERENCES users(id),
  created_at TEXT NOT NULL,
  UNIQUE(card_id,assignee_id)
);
CREATE INDEX IF NOT EXISTS assignments_assignee ON assignments(assignee_id,card_id);
CREATE TABLE IF NOT EXISTS watches (
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id),
  watching INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY(card_id,user_id)
);
CREATE TABLE IF NOT EXISTS pins (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL REFERENCES users(id),
  created_at TEXT NOT NULL,
  UNIQUE(user_id,card_id)
);
CREATE INDEX IF NOT EXISTS pins_user_recent ON pins(user_id,created_at DESC);
CREATE TABLE IF NOT EXISTS reactions (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  reactable_type TEXT NOT NULL CHECK(reactable_type IN ('Card','Comment')),
  reactable_id TEXT NOT NULL,
  reacter_id TEXT NOT NULL REFERENCES users(id),
  content TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS reactions_target ON reactions(reactable_type,reactable_id,created_at);
CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id),
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
  creator_id TEXT NOT NULL REFERENCES users(id),
  eventable_type TEXT NOT NULL CHECK(eventable_type IN ('Card','Comment')),
  eventable_id TEXT NOT NULL,
  action TEXT NOT NULL,
  particulars TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS events_account_recent ON events(account_id,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS events_board_recent ON events(board_id,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS events_eventable ON events(eventable_type,eventable_id);
CREATE TRIGGER IF NOT EXISTS cards_touch_board_insert AFTER INSERT ON cards BEGIN
  UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.board_id;
END;
CREATE TRIGGER IF NOT EXISTS cards_touch_board_update AFTER UPDATE ON cards BEGIN
  UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.board_id;
END;
CREATE TRIGGER IF NOT EXISTS cards_touch_board_delete AFTER DELETE ON cards BEGIN
  UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=OLD.board_id;
END;
