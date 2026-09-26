# Run with RAILS_ENV=test, DATABASE_URL pointing to a disposable Fizzy DB copy,
# and RIZZY_DISPOSABLE_SOURCE_DB=1. Prints observed Access timestamp effects.
require "json"

raise "A disposable Fizzy test database is required" unless Rails.env.test? && ENV["RIZZY_DISPOSABLE_SOURCE_DB"] == "1"

board, user = Board.find_each.lazy.filter_map do |candidate|
  candidate.account.users.where.not(role: :system).find_each.lazy.filter_map do |person|
    [ candidate, person ] unless Access.exists?(board: candidate, user: person)
  end.first
end.first
raise "No board/user pair without access" unless board && user
owner = board.account.users.find_by!(role: :owner)

result = Current.set(account: board.account, user: owner) do
  snapshot = -> { [user.reload.updated_at, board.reload.updated_at] }
  before = snapshot.call
  sleep 0.02
  access = Access.create!(account: board.account, board: board, user: user, involvement: :access_only)
  created = snapshot.call
  created_access_time = access.reload.updated_at
  sleep 0.02
  access.update!(involvement: :watching)
  changed = snapshot.call
  changed_access_time = access.reload.updated_at
  sleep 0.02
  access.update!(involvement: :watching)
  repeated = snapshot.call
  repeated_access_time = access.reload.updated_at
  sleep 0.02
  access.accessed
  accessed = snapshot.call
  accessed_access_time = access.reload.updated_at
  sleep 0.02
  access.accessed
  recent_access = snapshot.call
  recent_access_time = access.reload.updated_at
  sleep 0.02
  access.destroy!
  deleted = snapshot.call
  {
    create_touches_user: created[0] > before[0],
    create_touches_board: created[1] > before[1],
    change_updates_access: changed_access_time > created_access_time,
    change_touches_user: changed[0] > created[0],
    change_touches_board: changed[1] > created[1],
    repeat_updates_access: repeated_access_time > changed_access_time,
    repeat_touches_user: repeated[0] > changed[0],
    repeat_touches_board: repeated[1] > changed[1],
    access_updates_access: accessed_access_time > repeated_access_time,
    access_touches_user: accessed[0] > repeated[0],
    access_touches_board: accessed[1] > repeated[1],
    recent_access_updates_access: recent_access_time > accessed_access_time,
    recent_access_touches_user: recent_access[0] > accessed[0],
    recent_access_touches_board: recent_access[1] > accessed[1],
    delete_touches_user: deleted[0] > recent_access[0],
    delete_touches_board: deleted[1] > recent_access[1]
  }
end
puts JSON.generate(result)
