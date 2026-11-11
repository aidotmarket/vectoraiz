declare const __REPOSITORY_LICENCE__: string;

export function licenceName(text: string): string {
  return text.split("\n", 1)[0].trim();
}

export const repositoryLicenceName = licenceName(__REPOSITORY_LICENCE__);
