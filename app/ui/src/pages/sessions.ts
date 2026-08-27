import { page as projectsPage } from "./projects";
import type { Page } from "../main";

export const page: Page = {
  render(root: HTMLElement): void {
    projectsPage.render(root); // 会话明细从项目下钻进入，共用实现
  },
};
