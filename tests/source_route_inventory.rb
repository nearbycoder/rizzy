require "json"

routes = Rails.application.routes.routes.filter_map do |route|
  controller = route.defaults[:controller]
  action = route.defaults[:action]
  next unless controller && action

  klass = "#{controller.camelize}Controller".safe_constantize
  next unless klass&.action_methods&.include?(action.to_s)

  {
    verb: route.verb.to_s,
    path: route.path.spec.to_s,
    controller: controller,
    action: action.to_s
  }
end

puts JSON.pretty_generate({ source_revision: `git rev-parse HEAD`.strip, routes: routes })
