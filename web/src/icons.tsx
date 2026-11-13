// Icons named by packs (work item types, request types) map to components here.
// Unknown names fall back to a neutral circle, so a custom type never breaks the page.
import * as L from "lucide-react";
import type { ComponentType, SVGProps } from "react";

type IconC = ComponentType<SVGProps<SVGSVGElement>>;

const BY_NAME: Record<string, IconC> = {
  "check-square": L.SquareCheck, flag: L.Flag, "arrow-right-circle": L.CircleArrowRight, layers: L.Layers, bookmark: L.Bookmark, bug: L.Bug,
  "git-pull-request": L.GitPullRequest, flask: L.FlaskConical, target: L.Target, scale: L.Scale, "alert-triangle": L.TriangleAlert, stamp: L.Stamp,
  briefcase: L.Briefcase, "file-text": L.FileText, truck: L.Truck, "alert-octagon": L.OctagonAlert, "shopping-cart": L.ShoppingCart,
  "clipboard-check": L.ClipboardCheck, siren: L.Siren, wrench: L.Wrench, "help-circle": L.CircleHelp, zap: L.Zap, search: L.Search, key: L.Key,
  laptop: L.Laptop, circle: L.Circle, "list-checks": L.ListChecks, users: L.Users, "message-square-warning": L.MessageSquareWarning,
  milestone: L.Milestone, "file-warning": L.FileWarning, lightbulb: L.Lightbulb,
};

export function NamedIcon({ name, ...rest }: { name: string } & SVGProps<SVGSVGElement>) {
  const C = BY_NAME[name] ?? L.Circle;
  return <C aria-hidden="true" {...rest} />;
}

export const ICON_NAMES = Object.keys(BY_NAME);
